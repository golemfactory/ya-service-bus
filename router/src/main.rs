use std::collections::{hash_map::Entry, HashMap};
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use tokio::codec::{FramedRead, FramedWrite};
use tokio::net::TcpListener;
use tokio::prelude::*;
use tokio::sync::mpsc;

use ya_sb_api::*;
use ya_sb_router::codec::{GsbMessage, GsbMessageDecoder, GsbMessageEncoder};

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

fn main() {
    let listen_addr = "127.0.0.1:8080".parse().unwrap();
    let listener = TcpListener::bind(&listen_addr).expect("Unable to bind TCP listener");
    let dispatcher = Arc::new(Mutex::new(MessageDispatcher::new()));
    let registered_endpoints = Arc::new(Mutex::new(HashMap::new()));
    let pending_calls = Arc::new(Mutex::new(HashMap::new()));

    let server = listener
        .incoming()
        .map_err(|e| eprintln!("Accept failed: {:?}", e))
        .for_each(move |sock| {
            let addr = sock.peer_addr().unwrap();
            println!("Accepted connection from {}", addr);

            let (reader, writer) = sock.split();
            let writer = FramedWrite::new(writer, GsbMessageEncoder {});
            let reader = FramedRead::new(reader, GsbMessageDecoder::new());
            dispatcher.lock().unwrap().register(addr.clone(), writer).unwrap();

            let send_dispatcher = dispatcher.clone();
            let unregister_dispatcher = dispatcher.clone();
            let registered_endpoints = registered_endpoints.clone();
            let pending_calls = pending_calls.clone();

            tokio::spawn(
                reader
                    .for_each(move |msg| {
                        match msg {
                            GsbMessage::RegisterRequest(msg) => {
                                println!("Received RegisterRequest from {} service_id = {}", addr, &msg.service_id);
                                if !msg.service_id.chars().all(char::is_alphanumeric) {
                                    let msg = RegisterReply {
                                        code: RegisterReplyCode::RegisterBadRequest as i32,
                                        message: "Illegal service ID".to_string(),
                                    };
                                    send_dispatcher.lock().unwrap().send_message(&addr, msg)?;
                                } else if registered_endpoints
                                    .lock()
                                    .unwrap()
                                    .contains_key(&msg.service_id)
                                {
                                    let msg = RegisterReply {
                                        code: RegisterReplyCode::RegisterConflict as i32,
                                        message: "Service ID already registered".to_string(),
                                    };
                                    send_dispatcher.lock().unwrap().send_message(&addr, msg)?;
                                } else {
                                    registered_endpoints
                                        .lock()
                                        .unwrap()
                                        .insert(msg.service_id, addr);
                                    let msg = RegisterReply {
                                        code: RegisterReplyCode::RegisteredOk as i32,
                                        message: "".to_string(),
                                    };
                                    send_dispatcher.lock().unwrap().send_message(&addr, msg)?;
                                }
                            }
                            GsbMessage::UnregisterRequest(msg) => {
                                println!("Received UnregisterRequest from {} service_id = {}", addr, &msg.service_id);
                                match registered_endpoints.lock().unwrap().get(&msg.service_id) {
                                    Some(listener_addr) if *listener_addr == addr => {
                                        registered_endpoints
                                            .lock()
                                            .unwrap()
                                            .remove(&msg.service_id);
                                        let msg = UnregisterReply {
                                            code: UnregisterReplyCode::UnregisteredOk as i32,
                                        };
                                        send_dispatcher.lock().unwrap().send_message(&addr, msg)?;
                                    }
                                    _ => {
                                        // Service not registered or registered by another process than the caller
                                        let msg = UnregisterReply {
                                            code: UnregisterReplyCode::NotRegistered as i32,
                                        };
                                        send_dispatcher.lock().unwrap().send_message(&addr, msg)?;
                                    }
                                }
                            }
                            GsbMessage::CallRequest(msg) => {
                                // TODO: Handle remote services (currently assuming address == service_id)
                                println!("Received CallRequest from {} caller = {} address = {} request_id = {}", addr, &msg.caller, &msg.address, &msg.request_id);
                                match registered_endpoints.lock().unwrap().get(&msg.address) {
                                    Some(listener_addr) => {
                                        if pending_calls
                                            .lock()
                                            .unwrap()
                                            .contains_key(&msg.request_id)
                                        {
                                            // Call with this ID already exists
                                            let msg = CallReply {
                                                request_id: msg.request_id,
                                                code: CallReplyCode::CallReplyBadRequest as i32,
                                                reply_type: CallReplyType::Full as i32,
                                                data: vec![],
                                            };
                                            send_dispatcher
                                                .lock()
                                                .unwrap()
                                                .send_message(&addr, msg)?;
                                        } else {
                                            pending_calls
                                                .lock()
                                                .unwrap()
                                                .insert(msg.request_id.clone(), addr);
                                            send_dispatcher
                                                .lock()
                                                .unwrap()
                                                .send_message(&listener_addr, msg)?;
                                        }
                                    }
                                    None => {
                                        // There is no service registered under given address
                                        let msg = CallReply {
                                            request_id: msg.request_id,
                                            code: CallReplyCode::CallReplyBadRequest as i32,
                                            reply_type: CallReplyType::Full as i32,
                                            data: vec![],
                                        };
                                        send_dispatcher.lock().unwrap().send_message(&addr, msg)?;
                                    }
                                }
                            }
                            GsbMessage::CallReply(msg) => {
                                println!("Received CallReply from {} request_id = {}", addr, &msg.request_id);
                                match pending_calls.lock().unwrap().get(&msg.request_id) {
                                    Some(caller_addr) => {
                                        println!("Forwarding reply to {}", &caller_addr);
                                        if msg.reply_type == (CallReplyType::Full as i32) {
                                            pending_calls.lock().unwrap().remove(&msg.request_id);
                                        }
                                        send_dispatcher
                                            .lock()
                                            .unwrap()
                                            .send_message(&caller_addr, msg)?;
                                    }
                                    None => eprintln!("Unknown request ID: {}", msg.request_id),
                                }
                            }
                            _ => eprintln!("Unexpected message received"),
                        }
                        Ok(())
                    })
                    .map_err(|e| eprintln!("Error occured parsing message: {:?}", e))
                    .and_then(move |_| {
                        unregister_dispatcher.lock().unwrap().unregister(&addr).unwrap();
                        future::ok(())
                    }),
            )
        });

    tokio::run(server);
}
