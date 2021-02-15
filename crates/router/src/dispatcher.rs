use futures::prelude::*;

use std::collections::{hash_map::Entry, HashMap};
use std::fmt::{Debug, Display};
use std::hash::Hash;

use tokio::sync::mpsc;

pub struct MessageDispatcher<A, M, E>
where
    A: Hash + Eq,
{
    senders: HashMap<A, mpsc::Sender<Result<M, E>>>,
}

impl<A, M, E> MessageDispatcher<A, M, E>
where
    A: Hash + Eq + Display,
    M: Send + 'static,
    E: Send + 'static + Debug,
{
    pub fn new() -> Self {
        MessageDispatcher {
            senders: HashMap::new(),
        }
    }

    pub fn register<B: Sink<M, Error = E> + Send + 'static>(
        &mut self,
        addr: A,
        sink: B,
    ) -> anyhow::Result<()> {
        match self.senders.entry(addr) {
            Entry::Occupied(entry) => anyhow::bail!("Sender already registered: {}", entry.key()),
            Entry::Vacant(entry) => {
                let (tx, rx) = mpsc::channel(1000);
                tokio::spawn(async move {
                    let mut rx = rx;
                    futures::pin_mut!(sink);
                    if let Err(e) = sink.send_all(&mut rx).await {
                        log::error!("Send failed: {:?}", e)
                    }
                    if let Err(e) = sink.close().await {
                        // Silence the Io::NotConnected error, they show after connection closed
                        // on macOS only.
                        if format!("{:?}", e).contains("NotConnected") {
                            log::debug!("Socket already closed: {:?}", e)
                        } else {
                            log::error!("Connection close failed: {:?}", e)
                        }
                    }
                });
                entry.insert(tx);
                Ok(())
            }
        }
    }

    pub fn unregister(&mut self, addr: &A) -> () {
        self.senders.remove(addr);
    }

    pub fn send_message<T>(&mut self, addr: &A, msg: T) -> anyhow::Result<()>
    where
        T: Into<M> + Debug,
    {
        match self.senders.get_mut(addr) {
            None => anyhow::bail!("Sender not registered: {}, msg: {:?}", addr, msg),
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

#[cfg(test)]
mod test {

    use futures::channel::mpsc::{self, Receiver, Sender};
    use futures::stream::StreamExt;

    use super::MessageDispatcher;

    #[tokio::test]
    async fn test_dispatch_ok() {
        let (tx, mut rx): (Sender<String>, Receiver<String>) = mpsc::channel(100);
        let mut dispatcher = MessageDispatcher::new();
        let addr = "test_addr".to_string();
        let msg = "test_msg";
        dispatcher.register(addr.clone(), tx).unwrap();
        dispatcher.send_message(&addr, msg.to_string()).unwrap();
        let recv_msg = rx.next().await.unwrap();
        assert_eq!(recv_msg, msg.to_string());
    }

    #[tokio::test]
    async fn test_dispatch_unregistered() {
        let (tx, _): (Sender<String>, Receiver<String>) = mpsc::channel(100);
        let mut dispatcher = MessageDispatcher::new();
        let addr = "test_addr".to_string();
        let msg = "test_msg";
        dispatcher.register(addr.clone(), tx).unwrap();
        dispatcher.unregister(&addr);
        dispatcher.send_message(&addr, msg.to_string()).unwrap_err(); // Should be error
    }
}
