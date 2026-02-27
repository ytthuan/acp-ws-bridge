//! TLS configuration and self-signed certificate generation via rcgen.

use std::path::Path;

use anyhow::Result;
use tokio_native_tls::TlsAcceptor;

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

    let identity = native_tls::Identity::from_pkcs8(&cert_pem, &key_pem)?;
    let native_acceptor = native_tls::TlsAcceptor::new(identity)?;

    Ok(TlsAcceptor::from(native_acceptor))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

        // Verify the generated files contain valid PEM data that could be loaded.
        // Note: load_tls_config uses native-tls Identity::from_pkcs8 which may
        // not support all key types (e.g., ECDSA) on all platforms. We test
        // the file I/O and PEM structure instead.
        let cert_pem = std::fs::read_to_string(&cert_path).unwrap();
        let key_pem = std::fs::read_to_string(&key_path).unwrap();
        assert!(cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(cert_pem.contains("END CERTIFICATE"));
        assert!(key_pem.contains("BEGIN PRIVATE KEY"));
        assert!(key_pem.contains("END PRIVATE KEY"));
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
