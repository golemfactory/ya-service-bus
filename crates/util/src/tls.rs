pub use hex::FromHexError as ParseError;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, PeerMisbehaved, SignatureScheme};
use sha2::{Digest, Sha224};
use std::fmt::{Debug, Formatter};
use std::str::FromStr;
use std::sync::Arc;

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
pub struct HashVerifier {
    v: Arc<rustls::crypto::CryptoProvider>,
    hash: CertHash,
}

impl HashVerifier {
    pub fn new(hash: CertHash) -> Self {
        let provider = rustls::crypto::ring::default_provider();
        let v = Arc::new(provider);
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
