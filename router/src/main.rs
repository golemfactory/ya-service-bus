use std::clone::Clone;
use std::collections::{hash_map::Entry, HashMap};
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use tokio::codec::{FramedRead, FramedWrite};
use tokio::net::TcpListener;
use tokio::prelude::*;
use tokio::sync::mpsc;

use failure::_core::fmt::Display;
use ya_sb_proto::{
    codec::{GsbMessage, GsbMessageDecoder, GsbMessageEncoder},
    *,
};

struct MessageDispatcher<A, B>
where
    A: Hash + Eq,
    B: Sink,
{
    senders: HashMap<A, mpsc::Sender<B::SinkItem>>,
}

impl<A, B> MessageDispatcher<A, B>
where
    A: Hash + Eq,
    B: Sink + Send + 'static,
    B::SinkItem: Send + 'static,
    B::SinkError: From<mpsc::error::RecvError> + Debug,
{
    fn new() -> Self {
        MessageDispatcher {
            senders: HashMap::new(),
        }
    }

    fn register(&mut self, addr: A, sink: B) -> failure::Fallible<()> {
        match self.senders.entry(addr) {
            Entry::Occupied(_) => Err(failure::err_msg("Sender already registered")),
            Entry::Vacant(entry) => {
                let (tx, rx) = mpsc::channel(1000);
                tokio::spawn(
                    sink.send_all(rx)
                        .map(|_| ())
                        .map_err(|e| eprintln!("Send failed: {:?}", e)),
                );
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
        T: Into<B::SinkItem>,
    {
        match self.senders.get_mut(addr) {
            None => Err(failure::err_msg("Sender not registered")),
            Some(sender) => {
                let sender = sender.clone();
                tokio::spawn(
                    sender
                        .send(msg.into())
                        .map(|_| ())
                        .map_err(|e| eprintln!("Send failed: {:?}", e)),
                );
                Ok(())
            }
        }
    }
}

type ServiceId = String;
type RequestId = String;

fn is_valid_service_id(service_id: &ServiceId) -> bool {
    service_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-')
}

struct Router<A, B>
where
    A: Hash + Eq,
    B: Sink,
{
    dispatcher: Arc<Mutex<MessageDispatcher<A, B>>>,
    registered_endpoints: Arc<Mutex<HashMap<ServiceId, A>>>,
    pending_calls: Arc<Mutex<HashMap<RequestId, A>>>,
}

impl<A, B> Clone for Router<A, B>
where
    A: Hash + Eq,
    B: Sink,
{
    fn clone(&self) -> Self {
        Router {
            dispatcher: self.dispatcher.clone(),
            registered_endpoints: self.registered_endpoints.clone(),
            pending_calls: self.pending_calls.clone(),
        }
    }
}

impl<A, B> Router<A, B>
where
    A: Hash + Eq + Display + Clone,
    B: Sink + Send + 'static,
    B::SinkItem: Send + 'static + From<GsbMessage>,
    B::SinkError: From<mpsc::error::RecvError> + Debug,
{
    fn new() -> Self {
        Router {
            dispatcher: Arc::new(Mutex::new(MessageDispatcher::new())),
            registered_endpoints: Arc::new(Mutex::new(HashMap::new())),
            pending_calls: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn connect(&mut self, addr: A, sink: B) -> failure::Fallible<()> {
        println!("Accepted connection from {}", addr);
        self.dispatcher.lock().unwrap().register(addr, sink)
    }

    fn disconnect(&mut self, addr: &A) -> failure::Fallible<()> {
        println!("Closed connection with {}", addr);
        self.dispatcher.lock().unwrap().unregister(addr)
        // TODO: Clean up registered endpoints and pending calls
    }

    fn send_message<T>(&mut self, addr: &A, msg: T) -> failure::Fallible<()>
    where
        T: Into<GsbMessage>,
    {
        self.dispatcher
            .lock()
            .unwrap()
            .send_message(addr, msg.into())
    }

    fn register_endpoint(&mut self, addr: &A, msg: RegisterRequest) -> failure::Fallible<()> {
        println!(
            "Received RegisterRequest from {}. service_id = {}",
            addr, &msg.service_id
        );
        let msg = if !is_valid_service_id(&msg.service_id) {
            RegisterReply {
                code: RegisterReplyCode::RegisterBadRequest as i32,
                message: "Illegal service ID".to_string(),
            }
        } else {
            match self
                .registered_endpoints
                .lock()
                .unwrap()
                .entry(msg.service_id)
            {
                Entry::Occupied(entry) => RegisterReply {
                    code: RegisterReplyCode::RegisterConflict as i32,
                    message: "Service ID already registered".to_string(),
                },
                Entry::Vacant(entry) => {
                    entry.insert(addr.clone());
                    RegisterReply {
                        code: RegisterReplyCode::RegisteredOk as i32,
                        message: "Service successfully registered".to_string(),
                    }
                }
            }
        };
        println!("{}", msg.message);
        self.send_message(addr, msg)
    }

    fn unregister_endpoint(&mut self, addr: &A, msg: UnregisterRequest) -> failure::Fallible<()> {
        println!(
            "Received UnregisterRequest from {}. service_id = {}",
            addr, &msg.service_id
        );
        let msg = match self
            .registered_endpoints
            .lock()
            .unwrap()
            .entry(msg.service_id)
        {
            Entry::Occupied(entry) if entry.get() == addr => {
                entry.remove();
                println!("Service successfully unregistered");
                UnregisterReply {
                    code: UnregisterReplyCode::UnregisteredOk as i32,
                }
            }
            _ => {
                println!("Service not registered or registered by another server");
                UnregisterReply {
                    code: UnregisterReplyCode::NotRegistered as i32,
                }
            }
        };
        self.send_message(addr, msg)
    }

    fn call(&mut self, caller_addr: &A, msg: CallRequest) -> failure::Fallible<()> {
        println!(
            "Received CallRequest from {}. caller = {}, address = {}, request_id = {}",
            caller_addr, &msg.caller, &msg.address, &msg.request_id
        );
        let server_addr = match self
            .pending_calls
            .lock()
            .unwrap()
            .entry(msg.request_id.clone())
        {
            Entry::Occupied(_) => Err("CallRequest with this ID already exists".to_string()),
            Entry::Vacant(call_entry) => {
                // TODO: Prefix matching
                match self.registered_endpoints.lock().unwrap().get(&msg.address) {
                    None => Err("No service registered under given address".to_string()),
                    Some(addr) => {
                        call_entry.insert(caller_addr.clone());
                        Ok(addr.clone())
                    }
                }
            }
        };
        match server_addr {
            Ok(server_addr) => {
                println!("Forwarding CallRequest to {}", server_addr);
                self.send_message(&server_addr, msg)
            }
            Err(err) => {
                println!("{}", err);
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
        println!(
            "Received CallReply from {} request_id = {}",
            server_addr, &msg.request_id
        );
        let caller_addr = match self
            .pending_calls
            .lock()
            .unwrap()
            .entry(msg.request_id.clone())
        {
            Entry::Occupied(entry) => {
                let caller_addr = entry.get().clone();
                if msg.reply_type == CallReplyType::Full as i32 {
                    entry.remove_entry();
                }
                Ok(caller_addr)
            }
            Entry::Vacant(_) => Err("Unknown request ID"),
        };
        match caller_addr {
            Ok(addr) => self.send_message(&addr, msg),
            Err(err) => Ok(println!("{}", err)),
        }
    }

    fn handle_message(&mut self, addr: A, msg: GsbMessage) -> failure::Fallible<()> {
        match msg {
            GsbMessage::RegisterRequest(msg) => self.register_endpoint(&addr, msg),
            GsbMessage::UnregisterRequest(msg) => self.unregister_endpoint(&addr, msg),
            GsbMessage::CallRequest(msg) => self.call(&addr, msg),
            GsbMessage::CallReply(msg) => self.reply(&addr, msg),
            _ => Err(failure::err_msg(format!(
                "Unexpected message received: {:?}",
                msg
            ))),
        }
    }
}

fn main() {
    let listen_addr = "127.0.0.1:8245".parse().unwrap();
    let listener = TcpListener::bind(&listen_addr).expect("Unable to bind TCP listener");
    let mut router = Router::new();

    let server = listener
        .incoming()
        .map_err(|e| eprintln!("Accept failed: {:?}", e))
        .for_each(move |sock| {
            let addr = sock.peer_addr().unwrap();
            let (reader, writer) = sock.split();
            let writer = FramedWrite::new(writer, GsbMessageEncoder {});
            let reader = FramedRead::new(reader, GsbMessageDecoder::new());

            router.connect(addr.clone(), writer);
            let mut router1 = router.clone();
            let mut router2 = router.clone();

            tokio::spawn(
                reader
                    .from_err()
                    .for_each(move |msg| future::done(router1.handle_message(addr.clone(), msg)))
                    .and_then(move |_| future::done(router2.disconnect(&addr)))
                    .map_err(|e| eprintln!("Error occurred handling message: {:?}", e)),
            )
        });

    tokio::run(server);
}
