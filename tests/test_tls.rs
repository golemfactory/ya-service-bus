#![cfg(feature = "tls")]

use std::sync::Arc;
use std::time::Duration;

use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use ya_sb_util::tls::CertHash;

async fn handshake(correct_pin: bool) {
    let certified = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = certified.cert.der().clone();
    let pin = if correct_pin {
        CertHash::from(&cert)
    } else {
        CertHash::from(&CertificateDer::from(vec![0]))
    };
    let key = PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()).into();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = async {
        let (socket, _) = listener.accept().await.unwrap();
        acceptor.accept(socket).await
    };
    let client = ya_service_bus::connection::tls(address, pin);
    let (server, client) = tokio::time::timeout(Duration::from_secs(5), async {
        futures::join!(server, client)
    })
    .await
    .expect("TLS handshake timed out");
    assert_eq!(client.is_ok(), correct_pin);
    assert_eq!(server.is_ok(), correct_pin);
}

#[actix_rt::test]
async fn tls_accepts_the_pinned_certificate() {
    handshake(true).await;
}

#[actix_rt::test]
async fn tls_rejects_a_different_certificate() {
    handshake(false).await;
}
