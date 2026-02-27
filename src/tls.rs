//! TLS configuration and self-signed certificate generation via rcgen.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

/// Generate a self-signed certificate and save to disk.
pub fn generate_self_signed_cert(
    cert_path: &Path,
    key_path: &Path,
    hostnames: &[String],
) -> Result<()> {
    use rcgen::generate_simple_self_signed;

    let subject_alt_names: Vec<String> = if hostnames.is_empty() {
        vec!["localhost".to_string(), "127.0.0.1".to_string()]
    } else {
        hostnames.to_vec()
    };

    let certified_key = generate_simple_self_signed(subject_alt_names)?;

    std::fs::write(cert_path, certified_key.cert.pem())?;
    std::fs::write(key_path, certified_key.key_pair.serialize_pem())?;

    Ok(())
}

/// Load TLS config from cert/key files and return a TlsAcceptor.
pub fn load_tls_config(cert_path: &str, key_path: &str) -> Result<TlsAcceptor> {
    let cert_pem = std::fs::read(cert_path)?;
    let key_pem = std::fs::read(key_path)?;

    let certs = rustls_pemfile::certs(&mut &*cert_pem).collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut &*key_pem)?
        .ok_or_else(|| anyhow::anyhow!("no private key found in {}", key_path))?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}
