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
use std::io;
use std::net::SocketAddr;

use futures::prelude::*;
use tokio::net::{TcpStream, ToSocketAddrs};

pub use config::RouterConfig;
pub use router::InstanceConfig;
#[cfg(unix)]
pub use unix::connect;
use ya_sb_proto::codec::{GsbMessage, GsbMessageCodec, ProtocolError};
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

#[cfg(feature = "tls")]
mod tls;

#[cfg(feature = "tls")]
pub use tls::*;
