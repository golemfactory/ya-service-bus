#![deny(missing_docs)]
//! # Gsb Router
//!
//! ```no_run
//! use ya_sb_router::{InstanceConfig, RouterConfig};
//!
//! #[actix_rt::main]
//! async fn main() {
//!     let mut config = RouterConfig::from_env();
//!     config.gc_interval_secs(60);
//!     InstanceConfig::new(config).run_url(None).await;
//! }
//!
//! ```
pub use config::RouterConfig;
pub use router::InstanceConfig;

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::str::FromStr;

use anyhow::Result;
use futures::prelude::*;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use prost::bytes::{BufMut, BytesMut};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio_tungstenite::*;
use tokio_util::codec::{Decoder, Encoder};

#[cfg(unix)]
pub use unix::connect;

use url::Url;
use ya_sb_proto::codec::{
    GsbMessage, GsbMessageCodec, GsbMessageDecoder, GsbMessageEncoder, ProtocolError,
};
use ya_sb_proto::*;

mod config;
mod connection;
mod router;

/// Starts in background new server instance on given tcp address.
pub async fn bind_tcp_router(addr: SocketAddr) -> Result<(), std::io::Error> {
    actix_rt::spawn(
        InstanceConfig::new(RouterConfig::from_env())
            .bind_tcp(addr)
            .await?,
    );
    Ok(())
}

#[cfg(unix)]
mod unix {
    use std::path::Path;

    use tokio::net::UnixStream;

    use super::*;

    #[doc(hidden)]
    pub async fn connect(
        gsb_addr: GsbAddr,
    ) -> (
        Box<dyn Sink<GsbMessage, Error = ProtocolError> + Unpin>,
        Box<dyn Stream<Item = Result<GsbMessage, ProtocolError>> + Unpin>,
    ) {
        match gsb_addr {
            GsbAddr::Tcp(addr) => {
                let (sink, stream) = tcp_connect(addr).await;
                (Box::new(sink), Box::new(stream))
            }
            GsbAddr::Unix(path) => {
                let (sink, stream) = unix_connect(path).await;
                (Box::new(sink), Box::new(stream))
            }
            GsbAddr::Ws(addr) => {
                let (sink, stream) = ws_connect(addr).await.expect("TODO");
                (Box::new(sink), Box::new(stream))
            }
        }
    }

    pub async fn unix_connect<P: AsRef<Path>>(
        path: P,
    ) -> (
        impl Sink<GsbMessage, Error = ProtocolError>,
        impl Stream<Item = Result<GsbMessage, ProtocolError>>,
    ) {
        let sock = UnixStream::connect(path).await.expect("Connect failed");
        let framed = tokio_util::codec::Framed::new(sock, GsbMessageCodec::default());
        framed.split()
    }
}

#[cfg(not(unix))]
#[doc(hidden)]
pub async fn connect(
    gsb_addr: GsbAddr,
) -> (
    Box<dyn Sink<GsbMessage, Error = ProtocolError> + Unpin>,
    Box<dyn Stream<Item = Result<GsbMessage, ProtocolError>> + Unpin>,
) {
    match gsb_addr {
        GsbAddr::Tcp(addr) => {
            let (sink, stream) = tcp_connect(addr).await;
            (Box::new(sink), Box::new(stream))
        }
        GsbAddr::Unix(_) => panic!("Unix sockets not supported on this OS"),
    }
}

/// Starts in background new server instance on given gsb address.
pub async fn bind_gsb_router(gsb_url: Option<url::Url>) -> io::Result<()> {
    let _ = actix_rt::spawn(
        InstanceConfig::new(RouterConfig::from_env())
            .bind_url(gsb_url)
            .await?,
    );
    Ok(())
}

#[doc(hidden)]
pub async fn tcp_connect(
    addr: impl ToSocketAddrs,
) -> (
    impl Sink<GsbMessage, Error = ProtocolError>,
    impl Stream<Item = Result<GsbMessage, ProtocolError>>,
) {
    let sock = TcpStream::connect(&addr).await.expect("Connect failed");
    let framed = tokio_util::codec::Framed::new(sock, GsbMessageCodec::default());
    framed.split()
}

#[doc(hidden)]
pub async fn ws_connect(
    addr: String,
) -> Result<(
    Pin<Box<dyn Sink<GsbMessage, Error = ProtocolError>>>,
    Pin<Box<dyn Stream<Item = Result<GsbMessage, ProtocolError>>>>,
)> {
    //TODO fix support of "wss"
    let addr = format!("ws://{addr}");
    let addr = Url::from_str(&addr)?;
    let (stream, _resp) = tokio_tungstenite::connect_async(addr).await?;
    let (sink, stream) = stream.split();
    let sink = sink.with(to_ws_msg);
    let stream = stream.then(from_ws_msg);
    Ok((Box::pin(sink), Box::pin(stream)))
}

async fn to_ws_msg(msg: GsbMessage) -> Result<tungstenite::Message, ProtocolError> {
    let mut encoder = GsbMessageEncoder::default();
    let mut bytes = BytesMut::new();
    encoder.encode(msg, &mut bytes)?;
    let bytes = Vec::from(bytes.as_ref());
    Ok(tungstenite::Message::Binary(bytes))
}

async fn from_ws_msg(
    msg: tokio_tungstenite::tungstenite::Result<tokio_tungstenite::tungstenite::Message>,
) -> Result<GsbMessage, ProtocolError> {
    log::debug!("Msg: {:?}", msg);
    match msg {
        Ok(msg) => match msg {
            tungstenite::Message::Text(_) => {
                log::info!("Got Text");
                todo!()
            },
            tungstenite::Message::Binary(msg) => {
                let mut decoder = GsbMessageDecoder::default();
                let mut bytes = BytesMut::new();
                bytes.put_slice(&msg);
                match decoder.decode(&mut bytes)? {
                    Some(msg) => return Ok(msg),
                    None => return Err(ProtocolError::RecvError),
                }
            }
            tungstenite::Message::Ping(_) => {
                log::info!("Got Ping");
                todo!()
            },
            tungstenite::Message::Pong(_) => {
                log::info!("Got Pong");
                todo!()
            },
            tungstenite::Message::Close(_) => {
                log::info!("Got Close");
                todo!()
            },
            tungstenite::Message::Frame(_) => {
                log::info!("Got Frame");
                todo!()
            },
        },
        Err(err) => match err {
            tungstenite::Error::ConnectionClosed => {
                log::error!("Got ConnectionClosed");
                todo!()
            },
            tungstenite::Error::AlreadyClosed => {
                log::error!("Got AlreadyClosed");
                todo!()
            },
            tungstenite::Error::Io(_) => {
                log::error!("Got Io");
                todo!()
            },
            tungstenite::Error::Tls(_) => {
                log::error!("Got Tls");
                todo!()
            },
            tungstenite::Error::Capacity(_) => {
                log::error!("Got Capacity");
                todo!()
            },
            tungstenite::Error::Protocol(_) => {
                log::error!("Got Protocol");
                todo!()
            },
            tungstenite::Error::SendQueueFull(_) => {
                log::error!("Got SendQueueFull");
                todo!()
            },
            tungstenite::Error::Utf8 => {
                log::error!("Got Utf8");
                todo!()
            },
            tungstenite::Error::Url(_) => {
                log::error!("Got Url");
                todo!()
            },
            tungstenite::Error::Http(_) => {
                log::error!("Got Http");
                todo!()
            },
            tungstenite::Error::HttpFormat(_) => {
                log::error!("Got HttpFormat");
                todo!()
            },
        },
    }
}
