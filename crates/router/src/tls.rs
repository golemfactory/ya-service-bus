use actix::dev::Stream;
use futures::{Sink, StreamExt};
pub use hex::FromHexError as ParseError;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, PeerMisbehaved, SignatureScheme};
use sha2::{Digest, Sha224};
use std::fmt::{Debug, Formatter};
use std::io;
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::{TcpStream, ToSocketAddrs};
use ya_sb_proto::codec::{GsbMessage, GsbMessageCodec, ProtocolError};

#[derive(Eq, PartialEq, Clone)]
/// Hash to verify server certificate.
pub struct CertHash {
    hash: [u8; 28],
}

impl Debug for CertHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(&self.hash))
    }
}

impl FromStr for CertHash {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut hash = [0u8; 28];

        hex::decode_to_slice(s, &mut hash)?;
        Ok(Self { hash })
    }
}

impl<'a, 'b> From<&'b CertificateDer<'a>> for CertHash {
    fn from(value: &'b CertificateDer<'a>) -> Self {
        let mut hash = [0u8; 28];
        let cert_hash = Sha224::digest(value.as_ref());

        hash.copy_from_slice(cert_hash.as_ref());

        Self { hash }
    }
}

#[derive(Debug)]
struct HashVerifier {
    v: Arc<rustls::crypto::CryptoProvider>,
    hash: CertHash,
}

impl HashVerifier {
    fn new(hash: CertHash) -> Self {
        let v = Arc::clone(rustls::crypto::CryptoProvider::get_default().unwrap());
        Self { v, hash }
    }
}

impl ServerCertVerifier for HashVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let hash = CertHash::from(end_entity);

        if hash != self.hash {
            return Err(CertificateError::NotValidForName.into());
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme.into())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.v.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ECDSA_NISTP256_SHA256]
    }
}

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
/// async move {
///     let cert_hash = "393479950594e7c676ba121033a677a1316f722460827e217c82d2b3".parse()?;
///     let (tx, rx) = tls_connect("18.185.178.4:7464".parse()?, cert_hash).await?;
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
