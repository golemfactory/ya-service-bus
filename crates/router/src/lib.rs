use futures::prelude::*;

use std::net::SocketAddr;

use tokio::net::TcpStream;

use ya_sb_proto::codec::{GsbMessage, GsbMessageCodec, ProtocolError};
use ya_sb_proto::*;

mod config;
mod connection;
mod router;

pub async fn bind_tcp_router(addr: SocketAddr) -> Result<(), std::io::Error> {
    router::bind_tcp_router(addr).await
}

#[cfg(unix)]
mod unix {

    use super::*;
    use std::path::Path;
    use tokio::net::UnixStream;

    pub async fn connect(
        gsb_addr: GsbAddr,
    ) -> (
        Box<dyn Sink<GsbMessage, Error = ProtocolError> + Unpin>,
        Box<dyn Stream<Item = Result<GsbMessage, ProtocolError>> + Unpin>,
    ) {
        match gsb_addr {
            GsbAddr::Tcp(addr) => {
                let (sink, stream) = tcp_connect(&addr).await;
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
pub async fn bind_gsb_router(gsb_url: Option<url::Url>) -> Result<(), std::io::Error> {
    match GsbAddr::from_url(gsb_url) {
        GsbAddr::Tcp(addr) => router::bind_tcp_router(addr).await,
        GsbAddr::Unix(_) => panic!("Unix sockets not supported on this OS"),
    }
}

#[cfg(unix)]
pub async fn bind_gsb_router(gsb_url: Option<url::Url>) -> Result<(), std::io::Error> {
    match GsbAddr::from_url(gsb_url) {
        GsbAddr::Tcp(addr) => router::bind_tcp_router(addr).await,
        GsbAddr::Unix(path) => router::bind_unix_router(path).await,
    }
}

#[cfg(not(unix))]
pub async fn connect(
    gsb_addr: GsbAddr,
) -> (
    Box<dyn Sink<GsbMessage, Error = ProtocolError> + Unpin>,
    Box<dyn Stream<Item = Result<GsbMessage, ProtocolError>> + Unpin>,
) {
    match gsb_addr {
        GsbAddr::Tcp(addr) => {
            let (sink, stream) = tcp_connect(&addr).await;
            (Box::new(sink), Box::new(stream))
        }
        GsbAddr::Unix(_) => panic!("Unix sockets not supported on this OS"),
    }
}

#[cfg(unix)]
pub use unix::connect;

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
