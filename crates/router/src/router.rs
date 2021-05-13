use super::config::RouterConfig;
use crate::connection::{Connection, DropConnection};
use actix::Addr;
use actix_rt::net::TcpStream;
use bitflags::_core::sync::atomic::AtomicU64;
use futures::{Future, Sink};
use parking_lot::RwLock;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{hash_map, HashMap};
use std::fmt::Debug;
use std::net::SocketAddr;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use uuid::Uuid;
use ya_sb_proto::codec::{GsbMessage, ProtocolError};
use ya_sb_proto::BroadcastRequest;
use ya_sb_proto::*;
use ya_sb_util::PrefixLookupBag;

const BCAST_BACKLOG: usize = 16;

pub type RouterRef<W, C> = Arc<RwLock<Router<W, C>>>;

pub struct InstanceConfig {
    config: RouterConfig,
    instance_id: uuid::Uuid,
    name: String,
    version: String,
}

impl InstanceConfig {
    pub fn new(config: RouterConfig) -> Self {
        InstanceConfig {
            config,
            instance_id: Uuid::new_v4(),
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub(crate) fn hello(&self) -> Hello {
        Hello {
            instance_id: self.instance_id.as_bytes().to_vec(),
            name: self.name.clone(),
            version: self.version.clone(),
            ..Default::default()
        }
    }

    fn new_router<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    >(
        self: &Arc<Self>,
    ) -> RouterRef<W, ConnInfo> {
        Arc::new(RwLock::new(Router {
            instance: self.clone(),
            registered_instances: Default::default(),
            registered_endpoints: Default::default(),
            topics: Default::default(),
        }))
    }

    #[inline]
    pub(super) fn high_buffer_mark(&self) -> usize {
        self.config.high_buffer_mark
    }

    pub(super) fn forward_timeout(&self) -> Duration {
        self.config.forward_timeout
    }
}

pub type IdBytes = Box<[u8]>;

pub struct Router<
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    ConnInfo: Debug + Unpin + 'static,
> {
    instance: Arc<InstanceConfig>,
    registered_instances: HashMap<IdBytes, Addr<Connection<W, ConnInfo>>>,
    registered_endpoints: PrefixLookupBag<Addr<Connection<W, ConnInfo>>>,
    topics: HashMap<String, broadcast::Sender<BroadcastRequest>>,
}

impl<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static, ConnInfo: Debug + Unpin>
    Router<W, ConnInfo>
{
    pub fn resolve_node(&self, service_id: &str) -> Option<Addr<Connection<W, ConnInfo>>> {
        self.registered_endpoints.get(service_id).cloned()
    }

    pub fn register_service(
        &mut self,
        service_id: String,
        connection: Addr<Connection<W, ConnInfo>>,
    ) -> bool {
        if let Some(prev_connection) = self
            .registered_endpoints
            .insert(service_id.clone(), connection)
        {
            self.registered_endpoints
                .insert(service_id, prev_connection);
            false
        } else {
            true
        }
    }

    pub fn unregister_service(
        &mut self,
        service_id: &str,
        connection: &Addr<Connection<W, ConnInfo>>,
    ) {
        if let Some(prev_addr) = self.registered_endpoints.remove(service_id) {
            if prev_addr != *connection && prev_addr.connected() {
                let _ = self
                    .registered_endpoints
                    .insert(service_id.into(), prev_addr);
                log::error!("attempt for unregister unowned service {}", service_id);
            }
        }
    }

    pub fn subscribe_topic(&mut self, topic_id: String) -> broadcast::Receiver<BroadcastRequest> {
        match self.topics.entry(topic_id) {
            hash_map::Entry::Vacant(v) => {
                let (tx, rx) = broadcast::channel(BCAST_BACKLOG);
                v.insert(tx);
                rx
            }
            hash_map::Entry::Occupied(o) => o.get().subscribe(),
        }
    }

    pub fn find_topic(&self, topic_id: &str) -> Option<broadcast::Sender<BroadcastRequest>> {
        self.topics.get(topic_id).map(Clone::clone)
    }

    pub fn new_connection(
        &mut self,
        instance_id: IdBytes,
        connection: Addr<Connection<W, ConnInfo>>,
    ) -> impl Future<Output = ()> + 'static {
        let fut = if let Some(old_connection) =
            self.registered_instances.insert(instance_id, connection)
        {
            Some(
                old_connection
                    .send(DropConnection)
                    .timeout(Duration::from_secs(10)),
            )
        } else {
            None
        };
        async move {
            if let Some(fut) = fut {
                if let Err(_e) = fut.await {
                    log::error!("unable to disconnect connection")
                }
            }
        }
    }

    pub fn remove_connection(
        &mut self,
        instance_id: IdBytes,
        connection: &Addr<Connection<W, ConnInfo>>,
    ) {
        if let Some(prev_connection) = self.registered_instances.remove(&instance_id) {
            if prev_connection != *connection {
                self.registered_instances
                    .insert(instance_id, prev_connection);
            }
        }
    }
}

pub async fn bind_tcp_router(addr: SocketAddr) -> Result<(), std::io::Error> {
    use actix_service::fn_service;
    let instance_config = Arc::new(InstanceConfig::new(RouterConfig::from_env()));
    let router = instance_config.new_router();

    let router_status = router.clone();

    log::info!("Router listening on: {}", addr);
    tokio::spawn(
        actix_server::ServerBuilder::new()
            .bind("gsb", addr, move || {
                let router = router.clone();
                let instance_config = instance_config.clone();

                fn_service(move |sock: TcpStream| {
                    let addr = sock.peer_addr().unwrap();
                    let (input, output) = sock.into_split();
                    let _connection = super::connection::connection(
                        instance_config.clone(),
                        router.clone(),
                        addr,
                        input,
                        output,
                    );
                    futures::future::ok::<_, anyhow::Error>(())
                })
            })?
            .run(),
    );
    tokio::spawn(router_gc_worker(router_status));
    Ok(())
}

#[cfg(unix)]
pub async fn bind_unix_router<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    use actix_service::fn_service;
    use tokio::net::UnixStream;
    let instance_config = Arc::new(InstanceConfig::new(RouterConfig::from_env()));
    let router = instance_config.new_router();

    let router_status = router.clone();
    let worker_counter = Arc::new(AtomicU64::new(1));

    tokio::spawn(
        actix_server::ServerBuilder::new()
            .bind_uds("gsb", path, move || {
                let router = router.clone();
                let instance_config = instance_config.clone();
                let request_counter = RefCell::new(0u64);
                let worker_id = worker_counter.fetch_add(1, std::sync::atomic::Ordering::AcqRel);

                fn_service(move |sock: UnixStream| {
                    let _addr = sock.peer_addr().unwrap();
                    let connection_id = {
                        let mut b = request_counter.borrow_mut();
                        *b += 1;
                        *b
                    };

                    let (input, output) = sock.into_split();
                    let _connection = super::connection::connection(
                        instance_config.clone(),
                        router.clone(),
                        (worker_id, connection_id),
                        input,
                        output,
                    );
                    futures::future::ok::<_, anyhow::Error>(())
                })
            })?
            .run(),
    );
    tokio::spawn(router_gc_worker(router_status));
    Ok(())
}

async fn router_gc_worker<
    S: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    C: Debug + Unpin + 'static,
>(
    router: RouterRef<S, C>,
) {
    let gc_interval = match router.read().instance.config.gc_interval.clone() {
        Some(interval) => interval,
        None => return,
    };

    loop {
        tokio::time::delay_for(gc_interval).await;
        let (num_of_connections, num_of_endpoints, num_of_topics) = {
            let r = router.read();
            (
                r.registered_instances.len(),
                r.registered_endpoints.len(),
                r.topics.len(),
            )
        };
        log::info!(
            "GSB status [connections:{}], [endpoints:{}], [topics:{}]",
            num_of_connections,
            num_of_endpoints,
            num_of_topics
        );
    }
}
