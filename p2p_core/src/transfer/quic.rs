use anyhow::Result;
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use super::constants::TRANSFER_PORT;

/// Generate a self-signed certificate for QUIC
pub fn generate_self_signed_cert()
-> Result<(Vec<CertificateDer<'static>>, PrivatePkcs8KeyDer<'static>)> {
    let certified_key = generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der());
    let cert_der = CertificateDer::from(certified_key.cert.der().to_vec());
    Ok((vec![cert_der], key))
}

/// Create a QUIC server endpoint
pub fn make_server_endpoint(bind_addr: SocketAddr) -> Result<Endpoint> {
    let (certs, key) = generate_self_signed_cert()?;

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key.into())?;

    server_crypto.alpn_protocols = vec![b"p2p-transfer".to_vec()];

    let mut server_config = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)?,
    ));

    // Configure transport
    let mut transport_config = TransportConfig::default();
    transport_config.max_idle_timeout(Some(Duration::from_secs(30).try_into()?));
    transport_config.keep_alive_interval(Some(Duration::from_secs(2)));
    // Optimize for throughput
    transport_config.stream_receive_window((10 * 1024 * 1024 as u32).into()); // 10 MiB
    transport_config.receive_window((20 * 1024 * 1024 as u32).into()); // 20 MiB
    transport_config.send_window(20 * 1024 * 1024);
    transport_config.datagram_receive_buffer_size(Some(20 * 1024 * 1024));

    server_config.transport_config(Arc::new(transport_config));

    let endpoint = Endpoint::server(server_config, bind_addr)?;
    Ok(endpoint)
}

/// Create a QUIC client endpoint (skip certificate verification for P2P)
pub fn make_client_endpoint() -> Result<Endpoint> {
    // Create client config that skips certificate verification (for self-signed certs)
    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    crypto.alpn_protocols = vec![b"p2p-transfer".to_vec()];

    let mut client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?,
    ));

    // Configure transport
    let mut transport_config = TransportConfig::default();
    transport_config.max_idle_timeout(Some(Duration::from_secs(30).try_into()?));
    transport_config.keep_alive_interval(Some(Duration::from_secs(2)));
    // Optimize for throughput
    transport_config.stream_receive_window((10 * 1024 * 1024 as u32).into()); // 10 MiB
    transport_config.receive_window((20 * 1024 * 1024 as u32).into()); // 20 MiB
    transport_config.send_window(20 * 1024 * 1024);
    transport_config.datagram_receive_buffer_size(Some(20 * 1024 * 1024));

    client_config.transport_config(Arc::new(transport_config));

    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    Ok(endpoint)
}

/// Custom certificate verifier that skips verification (for self-signed certs in P2P)
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}
