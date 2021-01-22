use chrono::{Duration, NaiveDateTime, Utc};
use futures::future::{AbortHandle, Abortable};
use futures::lock::Mutex;
use futures::prelude::*;
use lazy_static::lazy_static;
use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::env;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::*;

use ya_sb_proto::codec::{GsbMessage, GsbMessageCodec, ProtocolError};
use ya_sb_proto::*;
use ya_sb_util::PrefixLookupBag;
use std::ops::Not;

mod dispatcher;

lazy_static! {
    pub static ref GSB_PING_TIMEOUT: u64 = env::var("GSB_PING_TIMEOUT")
        .unwrap_or("60".into())
        .parse()
        .unwrap();
}

type ServiceId = String;
type RequestId = String;
type TopicId = String;

fn is_valid_service_id(service_id: &ServiceId) -> bool {
    service_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-')
}

fn is_valid_topic_id(topic_id: &TopicId) -> bool {
    topic_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

struct PendingCall<A> {
    caller_addr: A,
    service_id: ServiceId,
}

struct RawRouter<A: Hash + Eq, M, E> {
    dispatcher: dispatcher::MessageDispatcher<A, M, E>,
    registered_endpoints: PrefixLookupBag<A>,
    reversed_endpoints: HashMap<A, HashSet<ServiceId>>,
    pending_calls: HashMap<RequestId, PendingCall<A>>,
    client_calls: HashMap<A, HashSet<RequestId>>,
    endpoint_calls: HashMap<ServiceId, HashSet<RequestId>>,
    topic_subscriptions: HashMap<TopicId, HashSet<A>>,
    reversed_subscriptions: HashMap<A, HashSet<TopicId>>,
    connections: HashMap<Box<[u8]>, A>,
    last_seen: HashMap<A, NaiveDateTime>,
}

impl<A, M, E> RawRouter<A, M, E>
where
    A: Hash + Eq + Display + Clone,
    M: Send + From<GsbMessage> + 'static,
    E: Send + Debug + 'static,
{
    pub fn new() -> Self {
        RawRouter {
            dispatcher: dispatcher::MessageDispatcher::new(),
            registered_endpoints: PrefixLookupBag::default(),
            reversed_endpoints: HashMap::new(),
            pending_calls: HashMap::new(),
            client_calls: HashMap::new(),
            endpoint_calls: HashMap::new(),
            topic_subscriptions: HashMap::new(),
            reversed_subscriptions: HashMap::new(),
            last_seen: HashMap::new(),
            connections: Default::default(),
        }
    }

    pub fn connect<B: Sink<M, Error = E> + Send + 'static>(
        &mut self,
        addr: A,
        instance_id: &[u8],
        sink: B,
    ) {
        log::debug!("Accepted connection from {}", addr);
        self.dispatcher.register(addr.clone(), sink).unwrap();
        if !instance_id.is_empty() {
            if let Some(prev_addr) = self.connections.insert(instance_id.into(), addr.clone()) {
                self.disconnect(&prev_addr, None);
            }
        }
        self.last_seen.insert(addr, Utc::now().naive_utc());
    }

    pub fn disconnect(&mut self, addr: &A, instance_id : Option<&[u8]>) {
        log::debug!("Closing connection with {}", addr);
        self.last_seen.remove(addr);

        // IDs of all endpoints registered by this server
        let service_ids = self
            .reversed_endpoints
            .remove(addr)
            .unwrap_or(HashSet::new());

        for service_id in service_ids.iter() {
            log::debug!("unregistering service: {}", service_id);
            self.registered_endpoints.remove(service_id);
        }

        // IDs of all pending call requests unanswered by this server
        let pending_call_ids: Vec<RequestId> = service_ids
            .iter()
            .filter_map(|service_id| self.endpoint_calls.remove(service_id))
            .flatten()
            .collect();
        let pending_calls: Vec<(RequestId, PendingCall<A>)> = pending_call_ids
            .iter()
            .filter_map(|request_id| self.pending_calls.remove_entry(request_id))
            .collect();

        // Answer all pending calls with ServiceFailure reply
        for (request_id, pending_call) in pending_calls {
            self.client_calls
                .get_mut(&pending_call.caller_addr)
                .unwrap()
                .remove(&request_id);
            let msg = CallReply {
                request_id,
                code: CallReplyCode::ServiceFailure as i32,
                reply_type: CallReplyType::Full as i32,
                data: format!(
                    "Service {} call aborted due to its provider disconnection.",
                    pending_call.service_id
                )
                .into_bytes(),
            };
            self.send_message_safe(&pending_call.caller_addr, msg);
        }

        // Remove all pending calls coming from this client
        for request_id in self.client_calls.remove(addr).unwrap_or(HashSet::new()) {
            let pending_call = self.pending_calls.remove(&request_id).unwrap();
            self.endpoint_calls
                .get_mut(&pending_call.service_id)
                .unwrap()
                .remove(&request_id);
        }

        // Unsubscribe from all topics
        if let Some(topic_ids) = self.reversed_subscriptions.remove(addr) {
            for topic_id in topic_ids {
                self.topic_subscriptions
                    .get_mut(&topic_id)
                    .unwrap()
                    .remove(&addr);
            }
        }

        self.dispatcher.unregister(addr);
        if let Some(instance_id) = instance_id {
            if instance_id.is_empty().not() {
                if let Some(new_addr) = self.connections.remove(instance_id) {
                    if new_addr != *addr {
                        log::debug!("Replaced instance from {} to {}", addr, new_addr);
                        self.connections.insert(instance_id.into(), new_addr);
                    } else {
                        log::debug!("Removed instance for: {}", addr);
                    }
                }
            }
        }
        log::debug!("Connection with {} closed.", addr);
    }

    fn send_message<T>(&mut self, addr: &A, msg: T) -> anyhow::Result<()>
    where
        T: Into<GsbMessage>,
    {
        self.dispatcher.send_message(addr, msg.into())
    }

    fn send_message_safe<T>(&mut self, addr: &A, msg: T) -> ()
    where
        T: Into<GsbMessage>,
    {
        self.send_message(addr, msg)
            .unwrap_or_else(|err| log::error!("Send message failed: {:?}", err));
    }

    fn hello(&mut self, addr: &A, _msg: Hello) -> anyhow::Result<()> {
        log::warn!("unexpected hello from: {}", addr);
        Ok(())
    }

    fn register_endpoint(&mut self, addr: &A, msg: RegisterRequest) -> anyhow::Result<()> {
        log::trace!("{} is registering endpoint {}", addr, &msg.service_id);
        let msg = if !is_valid_service_id(&msg.service_id) {
            RegisterReply {
                code: RegisterReplyCode::RegisterBadRequest as i32,
                message: "Illegal service ID".to_string(),
            }
        } else {
            match self.registered_endpoints.entry(msg.service_id.clone()) {
                Entry::Occupied(_) => RegisterReply {
                    code: RegisterReplyCode::RegisterConflict as i32,
                    message: format!("Service ID '{}' already registered", msg.service_id),
                },
                Entry::Vacant(entry) => {
                    entry.insert(addr.clone());
                    self.reversed_endpoints
                        .entry(addr.clone())
                        .or_insert_with(|| HashSet::new())
                        .insert(msg.service_id.clone());
                    RegisterReply {
                        code: RegisterReplyCode::RegisteredOk as i32,
                        message: format!("Service ID '{}' successfully registered", msg.service_id),
                    }
                }
            }
        };
        log::trace!("register_endpoint msg: {}", msg.message);
        self.send_message(addr, msg)
    }

    fn unregister_endpoint(&mut self, addr: &A, msg: UnregisterRequest) -> anyhow::Result<()> {
        log::debug!(
            "Received UnregisterRequest from {}. service_id = {}",
            addr,
            &msg.service_id
        );
        let msg = match self.registered_endpoints.entry(msg.service_id.clone()) {
            Entry::Occupied(entry) if entry.get() == addr => {
                entry.remove();
                self.reversed_endpoints
                    .get_mut(addr)
                    .ok_or(anyhow::anyhow!("Address not found: {}", addr))?
                    .remove(&msg.service_id);
                log::debug!("Service successfully unregistered");
                UnregisterReply {
                    code: UnregisterReplyCode::UnregisteredOk as i32,
                }
            }
            _ => {
                log::warn!("Service not registered or registered by another server");
                UnregisterReply {
                    code: UnregisterReplyCode::NotRegistered as i32,
                }
            }
        };
        self.send_message(addr, msg)
    }

    fn call(&mut self, caller_addr: &A, msg: CallRequest) -> anyhow::Result<()> {
        log::debug!(
            "Received CallRequest from {}. caller = {}, address = {}, request_id = {}",
            caller_addr,
            &msg.caller,
            &msg.address,
            &msg.request_id
        );
        let server_addr = match self.pending_calls.entry(msg.request_id.clone()) {
            Entry::Occupied(_) => Err("CallRequest with this ID already exists".to_string()),
            Entry::Vacant(call_entry) => match self.registered_endpoints.get(&msg.address) {
                None => Err(format!(
                    "No service registered under given address '{}'.",
                    &msg.address
                )),
                Some(addr) => {
                    call_entry.insert(PendingCall {
                        caller_addr: caller_addr.clone(),
                        service_id: msg.address.clone(),
                    });
                    self.endpoint_calls
                        .entry(msg.address.clone())
                        .or_insert_with(|| HashSet::new())
                        .insert(msg.request_id.clone());
                    self.client_calls
                        .entry(caller_addr.clone())
                        .or_insert_with(|| HashSet::new())
                        .insert(msg.request_id.clone());
                    Ok(addr.clone())
                }
            },
        };
        match server_addr {
            Ok(server_addr) => {
                log::debug!("Forwarding CallRequest to {}", server_addr);
                self.send_message(&server_addr, msg)
            }
            Err(err) => {
                log::debug!("{}", err);
                let msg = CallReply {
                    request_id: msg.request_id,
                    code: CallReplyCode::CallReplyBadRequest as i32,
                    reply_type: CallReplyType::Full as i32,
                    data: err.into_bytes(),
                };
                self.send_message(caller_addr, msg)
            }
        }
    }

    fn reply(&mut self, server_addr: &A, msg: CallReply) -> anyhow::Result<()> {
        log::debug!(
            "Received CallReply from {} request_id = {}",
            server_addr,
            &msg.request_id
        );
        match self.pending_calls.entry(msg.request_id.clone()) {
            Entry::Occupied(entry) => {
                let pending_call = entry.get();
                let caller_addr = pending_call.caller_addr.clone();
                if msg.reply_type == CallReplyType::Full as i32 {
                    self.endpoint_calls
                        .get_mut(&pending_call.service_id)
                        .ok_or(anyhow::anyhow!(
                            "Service not found: {}",
                            pending_call.service_id
                        ))?
                        .remove(&msg.request_id);
                    self.client_calls
                        .get_mut(&pending_call.caller_addr)
                        .ok_or(anyhow::anyhow!(
                            "Caller not found: {}",
                            pending_call.caller_addr
                        ))?
                        .remove(&msg.request_id);
                    entry.remove_entry();
                }
                self.send_message(&caller_addr, msg)
            }
            Entry::Vacant(_) => Ok(log::error!(
                "Got reply for unknown request ID: {}",
                msg.request_id
            )),
        }
    }

    fn subscribe(&mut self, addr: &A, msg: SubscribeRequest) -> anyhow::Result<()> {
        log::debug!(
            "Received SubscribeRequest from {} topic = {}",
            addr,
            &msg.topic
        );
        let msg = if !is_valid_topic_id(&msg.topic) {
            SubscribeReply {
                code: SubscribeReplyCode::SubscribeBadRequest as i32,
                message: format!("Invalid topic ID: {}", msg.topic),
            }
        } else {
            if self
                .topic_subscriptions
                .entry(msg.topic.clone())
                .or_insert_with(|| HashSet::new())
                .insert(addr.clone())
            {
                self.reversed_subscriptions
                    .entry(addr.clone())
                    .or_insert_with(|| HashSet::new())
                    .insert(msg.topic);
                SubscribeReply {
                    code: SubscribeReplyCode::SubscribedOk as i32,
                    message: "Successfully subscribed to topic".to_string(),
                }
            } else {
                SubscribeReply {
                    code: SubscribeReplyCode::SubscribeBadRequest as i32,
                    message: "Already subscribed".to_string(),
                }
            }
        };
        log::trace!("subscribe msg: {}", msg.message);
        self.send_message(addr, msg)
    }

    fn unsubscribe(&mut self, addr: &A, msg: UnsubscribeRequest) -> anyhow::Result<()> {
        log::debug!(
            "Received UnsubscribeRequest from {} topic = {}",
            addr,
            &msg.topic
        );
        let msg = if self
            .topic_subscriptions
            .entry(msg.topic.clone())
            .or_insert_with(|| HashSet::new())
            .remove(addr)
        {
            self.reversed_subscriptions
                .get_mut(addr)
                .ok_or(anyhow::anyhow!("Address not found: {}", addr))?
                .remove(&msg.topic);
            log::debug!("Successfully unsubscribed");
            UnsubscribeReply {
                code: UnsubscribeReplyCode::UnsubscribedOk as i32,
            }
        } else {
            log::warn!("Addr {} not subscribed for topic: {}", addr, msg.topic);
            UnsubscribeReply {
                code: UnsubscribeReplyCode::NotSubscribed as i32,
            }
        };
        self.send_message(addr, msg)
    }

    fn broadcast(&mut self, addr: &A, msg: BroadcastRequest) -> anyhow::Result<()> {
        log::debug!(
            "Received BroadcastRequest from {} topic = {} caller = {}",
            addr,
            &msg.topic,
            &msg.caller,
        );
        let reply = if is_valid_topic_id(&msg.topic) {
            BroadcastReply {
                code: BroadcastReplyCode::BroadcastOk as i32,
                message: "OK".to_string(),
            }
        } else {
            BroadcastReply {
                code: BroadcastReplyCode::BroadcastBadRequest as i32,
                message: format!("Invalid topic ID: {}", msg.topic),
            }
        };
        self.send_message_safe(addr, reply);

        let subscribers = match self.topic_subscriptions.get(&msg.topic) {
            Some(subscribers) => subscribers.iter().map(|a| a.clone()).collect(),
            None => vec![],
        };
        subscribers.iter().for_each(|addr| {
            self.send_message_safe(addr, msg.clone());
        });

        Ok(())
    }

    fn ping(&mut self, addr: &A) -> anyhow::Result<()> {
        log::trace!("Sending ping to {}", addr);
        let ping = Ping {};
        self.send_message(addr, ping)
    }

    fn pong(&self, addr: &A) -> anyhow::Result<()> {
        log::trace!("Received pong from {}", addr);
        Ok(())
    }

    fn update_last_seen(&mut self, addr: &A) -> anyhow::Result<()> {
        if !self.last_seen.contains_key(addr) {
            anyhow::bail!("Client {} disconnected due to timeout", addr);
        }
        self.last_seen.insert(addr.clone(), Utc::now().naive_utc());
        Ok(())
    }

    fn clients_not_seen_since(&self, datetime: NaiveDateTime) -> Vec<A> {
        self.last_seen
            .iter()
            .filter(|(_, dt)| **dt < datetime)
            .map(|(addr, _)| addr.clone())
            .collect()
    }

    pub fn check_for_stale_connections(&mut self) {
        log::trace!("Checking for stale connections");
        let now = Utc::now().naive_utc();
        let timeout = Duration::seconds(*GSB_PING_TIMEOUT as i64);
        for addr in self.clients_not_seen_since(now - timeout * 2) {
            self.disconnect(&addr, None);
        }
        for addr in self.clients_not_seen_since(now - timeout) {
            self.ping(&addr)
                .unwrap_or_else(|e| log::error!("Error sending ping message {:?}", e))
        }
    }

    pub fn handle_message(&mut self, addr: A, msg: GsbMessage) -> anyhow::Result<()> {
        self.update_last_seen(&addr)?;
        match msg {
            GsbMessage::Hello(hello) => self.hello(&addr, hello),
            GsbMessage::RegisterRequest(msg) => self.register_endpoint(&addr, msg),
            GsbMessage::UnregisterRequest(msg) => self.unregister_endpoint(&addr, msg),
            GsbMessage::CallRequest(msg) => self.call(&addr, msg),
            GsbMessage::CallReply(msg) => self.reply(&addr, msg),
            GsbMessage::SubscribeRequest(msg) => self.subscribe(&addr, msg),
            GsbMessage::UnsubscribeRequest(msg) => self.unsubscribe(&addr, msg),
            GsbMessage::BroadcastRequest(msg) => self.broadcast(&addr, msg),
            GsbMessage::Pong(_) => self.pong(&addr),
            _ => anyhow::bail!("Unexpected message received: {:?}", msg),
        }
    }
}

pub struct Router<A, M, E>
where
    A: Hash + Eq,
{
    router: Arc<Mutex<RawRouter<A, M, E>>>,
    ping_abort_handle: AbortHandle,
    instance_id: uuid::Uuid,
    name: String,
    version: String,
}

impl<A, M, E> Router<A, M, E>
where
    A: Send + Sync + Hash + Eq + Display + Clone + 'static,
    M: Send + From<GsbMessage> + 'static,
    E: Send + Sync + Debug + 'static,
{
    pub fn new() -> Self {
        let instance_id = uuid::Uuid::new_v4();
        let router = Arc::new(Mutex::new(RawRouter::new()));
        let router1 = router.clone();
        let (ping_abort_handle, abort_registration) = AbortHandle::new_pair();

        // Spawn abortable periodic task: checking for stale connections (ping)
        tokio::spawn(Abortable::new(
            async move {
                loop {
                    // Check interval should be lower than ping timeout to avoid race conditions
                    let check_interval = std::time::Duration::from_secs(*GSB_PING_TIMEOUT) / 2;
                    tokio::time::delay_for(check_interval).await;
                    router1.lock().await.check_for_stale_connections();
                }
            },
            abort_registration,
        ));

        Router {
            router,
            ping_abort_handle,
            instance_id,
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn handle_connection<R, W>(&self, addr: A, mut reader: R, mut writer: W)
    where
        R: TryStream<Ok = GsbMessage> + Send + Unpin + 'static,
        R::Error: Into<anyhow::Error>,
        W: Sink<M, Error = E> + Send + Unpin + 'static,
    {
        let router = self.router.clone();
        let hello: Hello = Hello {
            instance_id: self.instance_id.as_bytes().to_vec(),
            name: self.name.clone(),
            version: self.version.clone(),
            ..Default::default()
        };

        tokio::spawn(async move {
            if let Err(e) = writer.send(From::from(GsbMessage::Hello(hello))).await {
                log::warn!("failed to send hello {:?}", e);
                return;
            }

            let hello = match reader.try_next().await {
                Ok(None) => {
                    log::warn!("Port scan from: {}", addr);
                    return;
                }
                Err(e) => {
                    handle_message_error(e.into());
                    return;
                }
                Ok(Some(GsbMessage::Hello(h))) => h,
                Ok(Some(other)) => {
                    log::error!("Expected hello from: {}, got: {:?}", addr, other);
                    return;
                }
            };

            router
                .lock()
                .await
                .connect(addr.clone(), &hello.instance_id, writer);

            reader
                .err_into()
                .try_for_each(|msg: GsbMessage| async {
                    router.lock().await.handle_message(addr.clone(), msg)
                })
                .await
                .unwrap_or_else(|e| handle_message_error(e));

            router.lock().await.disconnect(&addr, Some(&hello.instance_id));
        });
    }

    pub async fn handle_connection_stream<S, R, W>(&self, conn_stream: S)
    where
        S: TryStream<Ok = (A, R, W)>,
        S::Error: Debug,
        R: TryStream<Ok = GsbMessage> + Send + Unpin + 'static,
        R::Error: Into<anyhow::Error>,
        W: Sink<M, Error = E> + Send + Unpin + 'static,
    {
        conn_stream
            .into_stream()
            .for_each(|item| {
                match item {
                    Ok((addr, reader, writer)) => self.handle_connection(addr, reader, writer),
                    Err(e) => log::error!("Connection stream error: {:?}", e),
                }
                future::ready(())
            })
            .await;
    }
}

fn handle_message_error(e: anyhow::Error) {
    match e.root_cause().downcast_ref::<std::io::Error>() {
        Some(err) => {
            if err.kind() == std::io::ErrorKind::ConnectionReset {
                log::trace!("Ignoring TCP connection reset from GSB client...");
            } else {
                log::error!("Error handling messages: {:?}", err)
            }
        }
        None => log::error!("Error handling messages: {:?}", e),
    }
}

impl<A, M, E> Drop for Router<A, M, E>
where
    A: Hash + Eq,
{
    fn drop(&mut self) {
        self.ping_abort_handle.abort();
    }
}

pub async fn bind_gsb_router(gsb_url: Option<url::Url>) -> Result<(), std::io::Error> {
    bind_tcp_router(gsb_addr(gsb_url)).await
}

pub async fn bind_tcp_router(addr: SocketAddr) -> Result<(), std::io::Error> {
    let mut listener = TcpListener::bind(&addr)
        .map_err(|e| {
            log::error!("Failed to bind TCP listener at {}: {}", addr, e);
            e
        })
        .await?;

    let router = Router::new();
    log::info!("Router listening on: {}", addr);

    tokio::spawn(async move {
        let conn_stream = listener.incoming().map_ok(|sock| {
            let addr = sock.peer_addr().unwrap();
            let (writer, reader) = Framed::new(sock, GsbMessageCodec::default()).split();
            (addr, reader, writer)
        });
        router.handle_connection_stream(conn_stream).await;
    });
    Ok(())
}

pub async fn tcp_connect(
    addr: &SocketAddr,
) -> (
    impl Sink<GsbMessage, Error = ProtocolError>,
    impl Stream<Item = Result<GsbMessage, ProtocolError>>,
) {
    let sock = TcpStream::connect(&addr).await.expect("Connect failed");
    let framed = tokio_util::codec::Framed::new(sock, GsbMessageCodec::default());
    framed.split()
}
