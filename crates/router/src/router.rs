use std::collections::{hash_map, HashMap};
use std::fmt::{Debug, Display};
use std::future::Future;
use std::io;
use std::net::ToSocketAddrs;
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use actix::prelude::*;
use actix::Addr;
use actix_rt::net::TcpStream;
use actix_service::fn_service;
use futures::prelude::*;
use parking_lot::RwLock;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::broadcast;
use tokio::time::sleep;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use ya_sb_proto::codec::{GsbMessage, ProtocolError};
use ya_sb_proto::*;
use ya_sb_util::PrefixLookupBag;

use crate::connection::{Connection, DropConnection, GetNodeInfo};
use crate::web::RestManager;

use super::config::RouterConfig;

pub type RouterRef<W, C> = Arc<RwLock<Router<W, C>>>;

/// Router config with instance identification info.
#[derive(Clone)]
pub struct InstanceConfig {
    config: RouterConfig,
    instance_id: uuid::Uuid,
    name: String,
    version: String,
    router: Arc<RwLock<Option<Arc<dyn AbstractRouter>>>>,
}

impl InstanceConfig {
    /// Default instance identification
    pub fn new(config: RouterConfig) -> Self {
        InstanceConfig {
            config,
            instance_id: Uuid::new_v4(),
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            router: Arc::new(RwLock::new(None)),
        }
    }

    /// Instance with custom application name and version.
    pub fn with_app(
        config: RouterConfig,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        InstanceConfig {
            config,
            instance_id: Uuid::new_v4(),
            name: name.into(),
            version: version.into(),
            router: Arc::new(RwLock::new(None)),
        }
    }

    pub(crate) fn hello(&self) -> Hello {
        Hello {
            instance_id: self.instance_id.as_bytes().to_vec(),
            name: self.name.clone(),
            version: self.version.clone(),
        }
    }

    fn new_router<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    >(
        &self,
    ) -> RouterRef<W, ConnInfo> {
        let (node_trace_tx, _) = broadcast::channel(1000);
        Arc::new(RwLock::new(Router {
            config: self.config.clone(),
            registered_instances: Default::default(),
            registered_endpoints: Default::default(),
            topics: Default::default(),
            node_trace_tx,
        }))
    }

    fn create_router_and_save<
        W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
        ConnInfo: Debug + Unpin + 'static,
    >(
        &mut self,
    ) -> RouterRef<W, ConnInfo> {
        if self.router.read().is_some() {
            log::warn!("Router already existed. Overwriting it.");
        }

        let router = self.new_router();
        *self.router.write() = Some(Arc::new(router.clone()));
        router
    }

    #[inline]
    pub(super) fn high_buffer_mark(&self) -> usize {
        self.config.high_buffer_mark
    }

    pub(super) fn forward_timeout(&self) -> Duration {
        self.config.forward_timeout
    }

    pub(super) fn ping_interval(&self) -> Duration {
        self.config.ping_interval
    }

    pub(super) fn ping_timeout(&self) -> Duration {
        self.config.ping_timeout
    }

    #[cfg(feature = "tls")]
    /// Starts new tls server instance on given tcp address.
    pub async fn bind_tls<A: ToSocketAddrs + Display>(
        mut self,
        addr: A,
    ) -> io::Result<impl Future<Output = ()>> {
        let router = self.create_router_and_save();
        let instance_config = Arc::new(self);
        let router_status = router.clone();
        let tls_config = instance_config.config.tls.clone().map(Arc::new);

        log::info!("TLS Router listening on: {}", addr);
        let server = actix_server::ServerBuilder::new()
            .bind("gsb-tls", addr, move || {
                let router = router.clone();
                let instance_config = instance_config.clone();
                let acceptor: tokio_rustls::TlsAcceptor = tls_config
                    .clone()
                    .map(tokio_rustls::TlsAcceptor::from)
                    .unwrap();

                fn_service(move |sock: TcpStream| {
                    let acceptor = acceptor.clone();
                    let instance_config = instance_config.clone();
                    let router = router.clone();
                    async move {
                        let addr = sock.peer_addr()?;
                        let tls_sock = acceptor.accept(sock).await?;
                        let (tls_input, tls_output) = tokio::io::split(tls_sock);
                        let _connection = super::connection::connection(
                            instance_config.clone(),
                            router.clone(),
                            addr.clone(),
                            tls_input,
                            tls_output,
                        );

                        Ok::<_, anyhow::Error>(())
                    }
                })
            })?
            .run();
        let worker = router_gc_worker(server, router_status);
        Ok(worker)
    }

    /// Starts new server instance on given tcp address.
    pub async fn bind_tcp<A: ToSocketAddrs + Display>(
        mut self,
        addr: A,
    ) -> io::Result<impl Future<Output = ()>> {
        let router = self.create_router_and_save();
        let instance_config = Arc::new(self);
        let router_status = router.clone();

        log::info!("Router listening on: {}", addr);
        let server = actix_server::ServerBuilder::new()
            .bind("gsb", addr, move || {
                let router = router.clone();
                let instance_config = instance_config.clone();

                fn_service(move |sock: TcpStream| {
                    let instance_config = instance_config.clone();
                    let router = router.clone();
                    async move {
                        let addr = sock.peer_addr()?;
                        let (input, output) = sock.into_split();

                        let _connection =
                            super::connection::connection::<OwnedReadHalf, OwnedWriteHalf, _>(
                                instance_config.clone(),
                                router.clone(),
                                addr.clone(),
                                input,
                                output,
                            );
                        Ok::<_, anyhow::Error>(())
                    }
                })
            })?
            .run();
        let worker = router_gc_worker(server, router_status);
        Ok(worker)
    }

    #[cfg(unix)]
    /// Starts new server instance on given unix socket path address.
    pub async fn bind_unix(
        mut self,
        path: &Path,
    ) -> io::Result<impl Future<Output = ()> + 'static> {
        use std::cell::RefCell;
        use std::fmt;
        use std::sync::atomic::{AtomicU64, Ordering};
        use tokio::net::UnixStream;

        let router = self.create_router_and_save();
        let instance_config = Arc::new(self);
        let router_status = router.clone();
        let worker_counter = Arc::new(AtomicU64::new(1));

        // Display connection info as W<worker-id>C<connection-id-worker>
        struct ConnInfo(u64, u64);
        impl Debug for ConnInfo {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "W{:02}C{:04}", self.0, self.1)
            }
        }

        let server = actix_server::ServerBuilder::new()
            .bind_uds("gsb", path, move || {
                let router = router.clone();
                let instance_config = instance_config.clone();
                let request_counter = RefCell::new(0u64);
                let worker_id = worker_counter.fetch_add(1, Ordering::AcqRel);

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
                        ConnInfo(worker_id, connection_id),
                        input,
                        output,
                    );
                    futures::future::ok::<_, anyhow::Error>(())
                })
            })?
            .run();
        let worker = router_gc_worker(server, router_status);

        Ok(worker)
    }

    #[cfg(not(unix))]
    /// Starts new server instance on given gsb url address.
    pub async fn bind_url(
        self,
        gsb_url: Option<url::Url>,
    ) -> io::Result<impl Future<Output = ()> + 'static> {
        match GsbAddr::from_url(gsb_url) {
            GsbAddr::Tcp(addr) => self.bind_tcp(addr).await,
            GsbAddr::Unix(_) => panic!("Unix sockets not supported on this OS"),
        }
    }

    #[cfg(unix)]
    /// Starts new server instance on given gsb url address.
    pub async fn bind_url(
        self,
        gsb_url: Option<url::Url>,
    ) -> io::Result<impl Future<Output = ()> + 'static> {
        Ok(match GsbAddr::from_url(gsb_url) {
            GsbAddr::Tcp(addr) => {
                #[cfg(feature = "tls")]
                if self.config.tls.is_some() {
                    self.bind_tls(addr).await?.boxed()
                } else {
                    self.bind_tcp(addr).await?.boxed()
                }
                #[cfg(not(feature = "tls"))]
                self.bind_tcp(addr).await?.boxed()
            }
            GsbAddr::Unix(path) => self.bind_unix(&path).await?.boxed(),
        })
    }

    /// Starts new server instance on given gsb url address and waits until it stops.
    pub async fn run_router(
        self,
        gsb_url: impl Into<Option<url::Url>>,
        web_addr: impl Into<Option<url::Url>>,
    ) -> anyhow::Result<()> {
        let gsb_fut = self.clone().bind_url(gsb_url.into()).await?;

        if let Some(web_addr) = web_addr.into() {
            let web_fut = self.run_rest_api(web_addr).await?;
            let (_, web_result) = tokio::join!(gsb_fut, web_fut);
            web_result?;
        } else {
            gsb_fut.await;
        }

        Ok(())
    }

    /// Starts only the REST API server without the GSB server.
    pub async fn run_rest_api(self, web_addr: url::Url) -> anyhow::Result<JoinHandle<()>> {
        let router = self
            .router
            .read()
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Router not initialized"))?;
        let rest_manager = Arc::new(RestManager::new(router));

        Ok(actix_rt::spawn(async move {
            if let Err(e) = rest_manager.start_web_server(web_addr).await {
                log::error!("REST API server error: {}", e);
            }
        }))
    }
}

pub type IdBytes = Box<[u8]>;

#[derive(Clone)]
pub enum NodeEvent {
    New(String),
    Lost(String),
    Lag(u64),
}

/// Abstract router trait with only the functions used by web.rs
pub trait AbstractRouter: Send + Sync {
    fn node_events(&self) -> broadcast::Receiver<NodeEvent>;
    fn registered_instances_count(&self) -> usize;
    fn registered_endpoints_count(&self) -> usize;
    fn topics_count(&self) -> usize;
    fn get_connection_recipients(&self) -> Vec<actix::prelude::Recipient<GetNodeInfo>>;
}

pub struct Router<
    W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    ConnInfo: Debug + Unpin + 'static,
> {
    config: RouterConfig,
    registered_instances: HashMap<IdBytes, Addr<Connection<W, ConnInfo>>>,
    registered_endpoints: PrefixLookupBag<Addr<Connection<W, ConnInfo>>>,
    topics: HashMap<String, broadcast::Sender<BroadcastRequest>>,
    node_trace_tx: broadcast::Sender<NodeEvent>,
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
    ) -> bool {
        if let Some(prev_addr) = self.registered_endpoints.remove(service_id) {
            if prev_addr != *connection && prev_addr.connected() {
                let _ = self
                    .registered_endpoints
                    .insert(service_id.into(), prev_addr);
                log::error!("attempt for unregister unowned service {}", service_id);
            } else {
                return true;
            }
        }
        false
    }

    pub fn subscribe_topic(&mut self, topic_id: String) -> BroadcastStream<BroadcastRequest> {
        let rx = match self.topics.entry(topic_id) {
            hash_map::Entry::Vacant(v) => {
                let (tx, rx) = broadcast::channel(self.config.broadcast_backlog);
                v.insert(tx);
                rx
            }
            hash_map::Entry::Occupied(o) => o.get().subscribe(),
        };
        BroadcastStream::new(rx)
    }

    pub fn find_topic(&self, topic_id: &str) -> Option<broadcast::Sender<BroadcastRequest>> {
        self.topics.get(topic_id).map(Clone::clone)
    }

    pub fn new_connection(
        &mut self,
        instance_id: IdBytes,
        connection: Addr<Connection<W, ConnInfo>>,
    ) -> impl Future<Output = ()> + 'static {
        let fut = self
            .registered_instances
            .insert(instance_id.clone(), connection)
            .map(|old_connection| {
                old_connection
                    .send(DropConnection)
                    .timeout(Duration::from_secs(10))
            });

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

    /// Send a node event
    pub fn send_node_event(
        &self,
        event: NodeEvent,
    ) -> Result<usize, broadcast::error::SendError<NodeEvent>> {
        self.node_trace_tx.send(event)
    }
}

impl<W: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static, ConnInfo: Debug + Unpin>
    AbstractRouter for RouterRef<W, ConnInfo>
{
    fn node_events(&self) -> broadcast::Receiver<NodeEvent> {
        self.read().node_trace_tx.subscribe()
    }

    fn registered_instances_count(&self) -> usize {
        self.read().registered_instances.len()
    }

    fn registered_endpoints_count(&self) -> usize {
        self.read().registered_endpoints.len()
    }

    fn topics_count(&self) -> usize {
        self.read().topics.len()
    }

    fn get_connection_recipients(&self) -> Vec<Recipient<GetNodeInfo>> {
        self.read()
            .registered_instances
            .values()
            .map(|addr| addr.clone().recipient::<GetNodeInfo>())
            .collect()
    }
}

async fn router_gc_worker<
    S: Sink<GsbMessage, Error = ProtocolError> + Unpin + 'static,
    C: Debug + Unpin + 'static,
>(
    server: actix_server::Server,
    router: RouterRef<S, C>,
) {
    let (gc_interval, scan) = match router.read().config.gc_interval {
        Some(interval) => (interval, true),
        None => (Duration::from_secs(60), false),
    };

    let mut sc = server;
    loop {
        tokio::pin! {
            let sleep_fut = sleep(gc_interval);
        };
        match futures::future::select(sc, sleep_fut).await {
            future::Either::Left(_) => return,
            future::Either::Right((_, f)) => {
                sc = f;
            }
        };

        if scan {
            let (num_of_connections, num_of_endpoints, num_of_topics, empty_topics) = {
                let r = router.read();
                (
                    r.registered_instances.len(),
                    r.registered_endpoints.len(),
                    r.topics.len(),
                    r.topics
                        .iter()
                        .filter_map(|(topic_id, sender)| {
                            if sender.receiver_count() == 0 {
                                Some(topic_id.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>(),
                )
            };
            if !empty_topics.is_empty() {
                let mut rw = router.write();
                for topic_id in &empty_topics {
                    let _ = rw.topics.remove(topic_id);
                }
            }
            log::info!(
                "GSB status [connections:{}], [endpoints:{}], [topics:{}]",
                num_of_connections,
                num_of_endpoints,
                num_of_topics
            );
            if !empty_topics.is_empty() {
                log::info!("GSB removing unused topics {:?}", empty_topics);
            }
        }
    }
}
