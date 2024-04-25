use actix::dev::Stream;
use futures::{Sink, StreamExt};
use rustls::pki_types::ServerName;
use std::io;
use std::sync::Arc;
use tokio::net::{TcpStream, ToSocketAddrs};
use ya_sb_proto::codec::{GsbMessage, GsbMessageCodec, ProtocolError};
use ya_sb_util::tls::HashVerifier;
pub use ya_sb_util::tls::{CertHash, ParseError};

/// Connects to router using tls protocol
///
/// Important assumptions
///
/// We do not use any public certificate functions, we do not check the chain,
/// certificate expiration date or CLR entry.
///
/// The assumption is that the connecting person only knows the hash of the certificate
/// used by the server. This, of course, creates a problem when the server key leaks
/// and a new server needs to be set up. Therefore, all security is based on how
/// the configuration will be delivered to the node.
///
/// # Example
///
/// ```rust
/// use ya_sb_router::tls_connect;
/// use anyhow::Result;
///
/// async fn connnect() -> Result<()> {
///     let cert_hash = "393479950594e7c676ba121033a677a1316f722460827e217c82d2b3".parse()?;
///     let (_tx, _rx) = tls_connect("18.185.178.4:7464", cert_hash).await?;
///
///     todo!()
/// }
/// ```
///
pub async fn tls_connect(
    addr: impl ToSocketAddrs,
    cert_hash: CertHash,
) -> io::Result<(
    impl Sink<GsbMessage, Error = ProtocolError>,
    impl Stream<Item = Result<GsbMessage, ProtocolError>>,
)> {
    let v = Arc::new(HashVerifier::new(cert_hash));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(v)
            .with_no_client_auth(),
    ));
    let sock = TcpStream::connect(&addr).await?;
    let io = connector
        .connect(ServerName::IpAddress(sock.peer_addr()?.ip().into()), sock)
        .await?;
    let framed = tokio_util::codec::Framed::new(io, GsbMessageCodec::default());
    Ok(framed.split())
}
