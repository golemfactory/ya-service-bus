use futures::prelude::*;
use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::codec::*;

use ya_sb_proto::codec::{GsbMessage, GsbMessageCodec, ProtocolError};
use ya_sb_proto::*;
use ya_sb_util::PrefixLookupBag;

struct MessageDispatcher<A, M, E>
where
    A: Hash + Eq,
{
    senders: HashMap<A, mpsc::Sender<Result<M, E>>>,
}

impl<A, M, E> MessageDispatcher<A, M, E>
where
    A: Hash + Eq,
    M: Send + 'static,
    E: Send + 'static + From<mpsc::error::RecvError> + Debug,
{
    fn new() -> Self {
        MessageDispatcher {
            senders: HashMap::new(),
        }
    }

    fn register<B: Sink<M, Error = E> + Send + 'static>(
        &mut self,
        addr: A,
        sink: B,
    ) -> failure::Fallible<()> {
        match self.senders.entry(addr) {
            Entry::Occupied(_) => Err(failure::err_msg("Sender already registered")),
            Entry::Vacant(entry) => {
                let (tx, rx) = mpsc::channel(1000);
                tokio::spawn(async move {
                    let mut rx = rx;
                    futures::pin_mut!(sink);
                    if let Err(e) = sink.send_all(&mut rx).await {
                        log::error!("Send failed: {:?}", e)
                    }
                });
                entry.insert(tx);
                Ok(())
            }
        }
    }

    fn unregister(&mut self, addr: &A) -> failure::Fallible<()> {
        match self.senders.remove(addr) {
            None => Err(failure::err_msg("Sender not registered")),
            Some(_) => Ok(()),
        }
    }

    fn send_message<T>(&mut self, addr: &A, msg: T) -> failure::Fallible<()>
    where
        T: Into<M>,
    {
        match self.senders.get_mut(addr) {
            None => Err(failure::err_msg("Sender not registered")),
            Some(sender) => {
                let sender = sender.clone();
                let msg = msg.into();
                tokio::spawn(async move {
                    futures::pin_mut!(sender);
                    let _ = sender
                        .send(Ok(msg))
                        .await
                        .unwrap_or_else(|e| log::error!("Send message failed: {}", e));
                });
                Ok(())
            }
        }
    }
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

struct PendingCall<A>
where
    A: Hash + Eq,
{
    caller_addr: A,
    service_id: ServiceId,
}

pub struct Router<A, M, E>
where
    A: Hash + Eq,
{
    dispatcher: MessageDispatcher<A, M, E>,
    registered_endpoints: PrefixLookupBag<A>,
    reversed_endpoints: HashMap<A, HashSet<ServiceId>>,
    pending_calls: HashMap<RequestId, PendingCall<A>>,
    client_calls: HashMap<A, HashSet<RequestId>>,
    endpoint_calls: HashMap<ServiceId, HashSet<RequestId>>,
    topic_subscriptions: HashMap<TopicId, HashSet<A>>,
    reversed_subscriptions: HashMap<A, HashSet<TopicId>>,
}

impl<A, M, E> Router<A, M, E>
where
    A: Hash + Eq + Display + Clone,
    M: Send + 'static + From<GsbMessage>,
    E: Send + 'static + From<mpsc::error::RecvError> + Debug,
{
    pub fn new() -> Self {
        Router {
            dispatcher: MessageDispatcher::new(),
            registered_endpoints: PrefixLookupBag::default(),
            reversed_endpoints: HashMap::new(),
            pending_calls: HashMap::new(),
            client_calls: HashMap::new(),
            endpoint_calls: HashMap::new(),
            topic_subscriptions: HashMap::new(),
            reversed_subscriptions: HashMap::new(),
        }
    }

    pub fn connect<B: Sink<M, Error = E> + Send + 'static>(
        &mut self,
        addr: A,
        sink: B,
    ) -> failure::Fallible<()> {
        log::info!("Accepted connection from {}", addr);
        self.dispatcher.register(addr, sink)
    }

    pub fn disconnect(&mut self, addr: &A) -> failure::Fallible<()> {
        log::info!("Closed connection with {}", addr);
        self.dispatcher.unregister(addr)?;

        // IDs of all endpoints registered by this server
        let service_ids = match self.reversed_endpoints.entry(addr.clone()) {
            Entry::Occupied(entry) => entry.remove().into_iter().collect(),
            Entry::Vacant(_) => vec![],
        };

        service_ids.iter().for_each(|service_id| {
            self.registered_endpoints.remove(service_id);
        });

        // IDs of all pending call requests unanswered by this server
        let pending_call_ids: Vec<RequestId> = service_ids
            .into_iter()
            .filter_map(|service_id| match self.endpoint_calls.entry(service_id) {
                Entry::Occupied(entry) => Some(entry.remove().into_iter()),
                Entry::Vacant(_) => None,
            })
            .flatten()
            .collect();
        let pending_calls: Vec<(RequestId, PendingCall<A>)> = pending_call_ids
            .into_iter()
            .filter_map(|request_id| match self.pending_calls.entry(request_id) {
                Entry::Occupied(entry) => Some(entry.remove_entry()),
                Entry::Vacant(_) => None,
            })
            .collect();

        // Answer all pending calls with ServiceFailure reply
        pending_calls
            .into_iter()
            .for_each(|(request_id, pending_call)| {
                self.client_calls
                    .get_mut(&pending_call.caller_addr)
                    .unwrap()
                    .remove(&request_id);
                let msg = CallReply {
                    request_id,
                    code: CallReplyCode::ServiceFailure as i32,
                    reply_type: CallReplyType::Full as i32,
                    data: "Service disconnected".to_owned().into_bytes(),
                };
                self.send_message_safe(&pending_call.caller_addr, msg);
            });

        // Remove all pending calls coming from this client
        match self.client_calls.entry(addr.clone()) {
            Entry::Occupied(entry) => {
                entry.remove().drain().for_each(|request_id| {
                    let pending_call = self.pending_calls.remove(&request_id).unwrap();
                    self.endpoint_calls
                        .get_mut(&pending_call.service_id)
                        .unwrap()
                        .remove(&request_id);
                });
            }
            Entry::Vacant(_) => {}
        }

        // Unsubscribe from all topics
        match self.reversed_subscriptions.entry(addr.clone()) {
            Entry::Occupied(entry) => {
                entry.remove().drain().for_each(|topic_id| {
                    self.topic_subscriptions
                        .get_mut(&topic_id)
                        .unwrap()
                        .remove(&addr);
                });
            }
            Entry::Vacant(_) => {}
        }

        Ok(())
    }

    fn send_message<T>(&mut self, addr: &A, msg: T) -> failure::Fallible<()>
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

    fn register_endpoint(&mut self, addr: &A, msg: RegisterRequest) -> failure::Fallible<()> {
        log::info!(
            "Received RegisterRequest from {}. service_id = {}",
            addr,
            &msg.service_id
        );
        let msg = if !is_valid_service_id(&msg.service_id) {
            RegisterReply {
                code: RegisterReplyCode::RegisterBadRequest as i32,
                message: "Illegal service ID".to_string(),
            }
        } else {
            match self.registered_endpoints.entry(msg.service_id.clone()) {
                Entry::Occupied(_) => RegisterReply {
                    code: RegisterReplyCode::RegisterConflict as i32,
                    message: "Service ID already registered".to_string(),
                },
                Entry::Vacant(entry) => {
                    entry.insert(addr.clone());
                    self.reversed_endpoints
                        .entry(addr.clone())
                        .or_insert_with(|| HashSet::new())
                        .insert(msg.service_id);
                    RegisterReply {
                        code: RegisterReplyCode::RegisteredOk as i32,
                        message: "Service successfully registered".to_string(),
                    }
                }
            }
        };
        log::info!("{}", msg.message);
        self.send_message(addr, msg)
    }

    fn unregister_endpoint(&mut self, addr: &A, msg: UnregisterRequest) -> failure::Fallible<()> {
        log::info!(
            "Received UnregisterRequest from {}. service_id = {}",
            addr,
            &msg.service_id
        );
        let msg = match self.registered_endpoints.entry(msg.service_id.clone()) {
            Entry::Occupied(entry) if entry.get() == addr => {
                entry.remove();
                self.reversed_endpoints
                    .get_mut(addr)
                    .ok_or(failure::err_msg("Address not found"))?
                    .remove(&msg.service_id);
                log::info!("Service successfully unregistered");
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

    fn call(&mut self, caller_addr: &A, msg: CallRequest) -> failure::Fallible<()> {
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
                None => Err("No service registered under given address".to_string()),
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

    fn reply(&mut self, server_addr: &A, msg: CallReply) -> failure::Fallible<()> {
        log::debug!(
            "Received CallReply from {} request_id = {}",
            server_addr,
            &msg.request_id
        );
        let caller_addr = match self.pending_calls.entry(msg.request_id.clone()) {
            Entry::Occupied(entry) => {
                let pending_call = entry.get();
                let caller_addr = pending_call.caller_addr.clone();
                if msg.reply_type == CallReplyType::Full as i32 {
                    self.endpoint_calls
                        .get_mut(&pending_call.service_id)
                        .ok_or(failure::err_msg("Service not found"))?
                        .remove(&msg.request_id);
                    self.client_calls
                        .get_mut(&pending_call.caller_addr)
                        .ok_or(failure::err_msg("Client not found"))?
                        .remove(&msg.request_id);
                    entry.remove_entry();
                }
                Ok(caller_addr)
            }
            Entry::Vacant(_) => Err("Unknown request ID"),
        };
        match caller_addr {
            Ok(addr) => self.send_message(&addr, msg),
            Err(err) => Ok(log::error!("{}", err)),
        }
    }

    pub fn subscribe(&mut self, addr: &A, msg: SubscribeRequest) -> failure::Fallible<()> {
        log::info!(
            "Received SubscribeRequest from {} topic = {}",
            addr,
            &msg.topic
        );
        let msg = if !is_valid_topic_id(&msg.topic) {
            SubscribeReply {
                code: SubscribeReplyCode::SubscribeBadRequest as i32,
                message: "Invalid topic ID".to_string(),
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
        log::info!("{}", msg.message);
        self.send_message(addr, msg)
    }

    pub fn unsubscribe(&mut self, addr: &A, msg: UnsubscribeRequest) -> failure::Fallible<()> {
        log::info!(
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
                .ok_or(failure::err_msg("Address not found"))?
                .remove(&msg.topic);
            log::info!("Successfully unsubscribed");
            UnsubscribeReply {
                code: UnsubscribeReplyCode::UnsubscribedOk as i32,
            }
        } else {
            log::warn!("Not subscribed");
            UnsubscribeReply {
                code: UnsubscribeReplyCode::NotSubscribed as i32,
            }
        };
        self.send_message(addr, msg)
    }

    pub fn broadcast(&mut self, addr: &A, msg: BroadcastRequest) -> failure::Fallible<()> {
        log::debug!(
            "Received BroadcastRequest from {} topic = {}",
            addr,
            &msg.topic
        );
        let reply = if is_valid_topic_id(&msg.topic) {
            BroadcastReply {
                code: BroadcastReplyCode::BroadcastOk as i32,
                message: "OK".to_string(),
            }
        } else {
            BroadcastReply {
                code: BroadcastReplyCode::BroadcastBadRequest as i32,
                message: "Invalid topic ID".to_string(),
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

    pub fn handle_message(&mut self, addr: A, msg: GsbMessage) -> failure::Fallible<()> {
        match msg {
            GsbMessage::RegisterRequest(msg) => self.register_endpoint(&addr, msg),
            GsbMessage::UnregisterRequest(msg) => self.unregister_endpoint(&addr, msg),
            GsbMessage::CallRequest(msg) => self.call(&addr, msg),
            GsbMessage::CallReply(msg) => self.reply(&addr, msg),
            GsbMessage::SubscribeRequest(msg) => self.subscribe(&addr, msg),
            GsbMessage::UnsubscribeRequest(msg) => self.unsubscribe(&addr, msg),
            GsbMessage::BroadcastRequest(msg) => self.broadcast(&addr, msg),
            _ => Err(failure::err_msg(format!(
                "Unexpected message received: {:?}",
                msg
            ))),
        }
    }
}

pub async fn bind_router(addr: SocketAddr) -> Result<(), std::io::Error> {
    let mut listener = TcpListener::bind(&addr)
        .map_err(|e| {
            log::error!("Failed to bind TCP listener at {}: {}", addr, e);
            e
        })
        .await?;

    let router = Arc::new(Mutex::new(Router::new()));

    log::info!("Router listening on: {}", addr);

    let _ = tokio::spawn(async move {
        listener
            .incoming()
            .map_err(|e| {
                log::error!("Accept failed: {:?}", e);
                e
            })
            .try_for_each(move |sock| {
                let addr = sock.peer_addr().unwrap();
                let (writer, reader) = Framed::new(sock, GsbMessageCodec::default()).split();
                router
                    .lock()
                    .unwrap()
                    .connect(addr.clone(), writer)
                    .unwrap();
                let router1 = router.clone();
                let router2 = router.clone();

                let _ = tokio::spawn(
                    reader
                        .map_err(From::from)
                        .try_for_each(move |msg: GsbMessage| {
                            future::ready(router1.lock().unwrap().handle_message(addr.clone(), msg))
                        })
                        .and_then(move |_| future::ready(router2.lock().unwrap().disconnect(&addr)))
                        .map_err(|e| log::error!("Error occurred handling message: {:?}", e)),
                );
                future::ok(())
            })
            .await
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
