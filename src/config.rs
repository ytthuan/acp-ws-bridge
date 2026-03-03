//! Configuration types and loading.

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "acp-ws-bridge",
    about = "WebSocket bridge for GitHub Copilot CLI ACP"
)]
pub struct Config {
    /// WebSocket server listen port
    #[arg(long, default_value = "8765")]
    pub ws_port: u16,

    /// Copilot CLI ACP TCP port
    #[arg(long, default_value = "3000")]
    pub copilot_port: u16,

    /// Copilot CLI ACP TCP host
    #[arg(long, default_value = "127.0.0.1")]
    pub copilot_host: String,

    /// WebSocket listen address
    #[arg(long, default_value = "0.0.0.0")]
    pub listen_addr: String,

    /// TLS certificate path (enables TLS if set)
    #[arg(long)]
    pub tls_cert: Option<String>,

    /// TLS key path
    #[arg(long)]
    pub tls_key: Option<String>,

    /// Idle timeout in seconds (default 7 days)
    #[arg(long, default_value = "604800")]
    pub idle_timeout_secs: u64,

    /// Generate self-signed certificate and exit
    #[arg(long)]
    pub generate_cert: bool,

    /// Hostnames for self-signed certificate (comma-separated)
    #[arg(long, default_value = "localhost,127.0.0.1")]
    pub cert_hostnames: String,

    /// REST API listen port (default: ws_port + 1)
    #[arg(long)]
    pub api_port: Option<u16>,

    /// Log level
    #[arg(long, default_value = "info")]
    pub log_level: String,

    /// Path to the copilot CLI executable
    #[arg(long, default_value = "copilot")]
    pub copilot_path: String,

    /// Automatically spawn Copilot CLI as a child process
    #[arg(long, default_value = "true")]
    pub spawn_copilot: bool,

    /// Extra arguments to pass to Copilot CLI
    #[arg(long)]
    pub copilot_args: Vec<String>,

    /// Copilot CLI transport mode: "tcp" or "stdio".
    /// Auto-detected as "tcp" when --copilot-port is explicitly provided.
    #[arg(long)]
    pub copilot_mode: Option<String>,
}

impl Config {
    /// Resolved copilot mode: if explicitly set use that, otherwise auto-detect.
    /// When --copilot-port is provided without --copilot-mode, assume TCP.
    pub fn effective_copilot_mode(&self) -> &str {
        if let Some(ref mode) = self.copilot_mode {
            mode.as_str()
        } else {
            "stdio"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_default_config() {
        let config = Config::parse_from(["test"]);
        assert_eq!(config.ws_port, 8765);
        assert_eq!(config.copilot_port, 3000);
        assert_eq!(config.copilot_host, "127.0.0.1");
        assert_eq!(config.listen_addr, "0.0.0.0");
        assert_eq!(config.idle_timeout_secs, 604800);
        assert!(config.tls_cert.is_none());
        assert!(config.tls_key.is_none());
        assert!(config.api_port.is_none());
        assert!(!config.generate_cert);
        assert_eq!(config.cert_hostnames, "localhost,127.0.0.1");
        assert_eq!(config.log_level, "info");
        assert_eq!(config.copilot_path, "copilot");
        assert!(config.spawn_copilot);
        assert!(config.copilot_args.is_empty());
        assert!(config.copilot_mode.is_none());
        assert_eq!(config.effective_copilot_mode(), "stdio");
    }

    #[test]
    fn test_custom_config() {
        let config = Config::parse_from([
            "test",
            "--ws-port",
            "9999",
            "--copilot-port",
            "4000",
            "--copilot-host",
            "192.168.1.1",
            "--listen-addr",
            "127.0.0.1",
            "--idle-timeout-secs",
            "3600",
            "--tls-cert",
            "/tmp/cert.pem",
            "--tls-key",
            "/tmp/key.pem",
            "--log-level",
            "debug",
        ]);
        assert_eq!(config.ws_port, 9999);
        assert_eq!(config.copilot_port, 4000);
        assert_eq!(config.copilot_host, "192.168.1.1");
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.idle_timeout_secs, 3600);
        assert_eq!(config.tls_cert.as_deref(), Some("/tmp/cert.pem"));
        assert_eq!(config.tls_key.as_deref(), Some("/tmp/key.pem"));
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn test_api_port_default() {
        let config = Config::parse_from(["test"]);
        // api_port defaults to None, meaning ws_port + 1 is used at runtime
        assert!(config.api_port.is_none());
    }

    #[test]
    fn test_api_port_explicit() {
        let config = Config::parse_from(["test", "--api-port", "9000"]);
        assert_eq!(config.api_port, Some(9000));
    }

    #[test]
    fn test_generate_cert_flag() {
        let config = Config::parse_from(["test", "--generate-cert"]);
        assert!(config.generate_cert);
    }

    #[test]
    fn test_cert_hostnames_custom() {
        let config = Config::parse_from(["test", "--cert-hostnames", "example.com,10.0.0.1"]);
        assert_eq!(config.cert_hostnames, "example.com,10.0.0.1");
    }
}
