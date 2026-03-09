//! TLS configuration and self-signed certificate generation via rcgen.

use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tokio_rustls::rustls;
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

    // Restrict private key file permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

/// Load TLS config from cert/key files and return a TlsAcceptor.
pub fn load_tls_config(cert_path: &str, key_path: &str) -> Result<TlsAcceptor> {
    let cert_file = std::fs::File::open(cert_path)?;
    let key_file = std::fs::File::open(key_path)?;

    let mut cert_reader = BufReader::new(cert_file);
    let cert_chain =
        rustls_pemfile::certs(&mut cert_reader).collect::<std::result::Result<Vec<_>, _>>()?;
    if cert_chain.is_empty() {
        anyhow::bail!("No certificates found in {}", cert_path);
    }

    let mut key_reader = BufReader::new(key_file);
    let private_key = rustls_pemfile::private_key(&mut key_reader)?
        .ok_or_else(|| anyhow::anyhow!("No private key found in {}", key_path))?;

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)?;
    // Leave ALPN unset so the shared TLS config cannot negotiate HTTP/2 on the
    // WebSocket listener, which relies on HTTP/1.1 upgrade semantics.
    server_config.alpn_protocols.clear();

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_self_signed_cert() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        let hostnames = vec!["localhost".to_string(), "127.0.0.1".to_string()];

        generate_self_signed_cert(&cert_path, &key_path, &hostnames).unwrap();

        assert!(cert_path.exists());
        assert!(key_path.exists());

        // Verify files are non-empty and contain PEM markers
        let cert_content = std::fs::read_to_string(&cert_path).unwrap();
        let key_content = std::fs::read_to_string(&key_path).unwrap();
        assert!(cert_content.contains("BEGIN CERTIFICATE"));
        assert!(key_content.contains("BEGIN PRIVATE KEY"));
    }

    #[test]
    fn test_generate_cert_empty_hostnames_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        // Empty hostnames should fall back to localhost defaults
        generate_self_signed_cert(&cert_path, &key_path, &[]).unwrap();
        assert!(cert_path.exists());
        assert!(key_path.exists());
    }

    #[test]
    fn test_load_tls_config() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        let hostnames = vec!["localhost".to_string()];

        generate_self_signed_cert(&cert_path, &key_path, &hostnames).unwrap();

        let acceptor = load_tls_config(cert_path.to_str().unwrap(), key_path.to_str().unwrap());
        assert!(acceptor.is_ok());
    }

    #[test]
    fn test_load_tls_config_missing_files() {
        let result = load_tls_config("/nonexistent/cert.pem", "/nonexistent/key.pem");
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_cert_custom_hostnames() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");
        let hostnames = vec![
            "myhost.local".to_string(),
            "10.0.0.1".to_string(),
            "example.com".to_string(),
        ];

        generate_self_signed_cert(&cert_path, &key_path, &hostnames).unwrap();
        assert!(cert_path.exists());
        assert!(key_path.exists());
    }
}
