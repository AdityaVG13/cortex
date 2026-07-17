use rustls::ServerConfig;
use rustls_pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
fn default_tls_dir() -> PathBuf {
    crate::auth::cortex_dir().join("tls")
}
fn cert_path() -> PathBuf {
    std::env::var("CORTEX_TLS_CERT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_tls_dir().join("cert.pem"))
}
fn key_path() -> PathBuf {
    std::env::var("CORTEX_TLS_KEY")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_tls_dir().join("key.pem"))
}
pub fn try_load_tls() -> Result<Option<TlsAcceptor>, String> {
    let cert = cert_path();
    let key = key_path();
    if !cert.exists() && !key.exists() {
        return Ok(None);
    }
    if !cert.exists() {
        return Err(format!("TLS key found but cert missing at {}", cert.display()));
    }
    if !key.exists() {
        return Err(format!("TLS cert found but key missing at {}", key.display()));
    }
    let config = load_rustls_config(&cert, &key)?;
    Ok(Some(TlsAcceptor::from(Arc::new(config))))
}
fn load_rustls_config(cert_path: &Path, key_path: &Path) -> Result<ServerConfig, String> {
    let cert_file = std::fs::File::open(cert_path).map_err(|e| format!("Failed to open cert {}: {e}", cert_path.display()))?;
    let key_file = std::fs::File::open(key_path).map_err(|e| format!("Failed to open key {}: {e}", key_path.display()))?;
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_reader_iter(std::io::BufReader::new(cert_file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to parse certs: {e}"))?;
    if certs.is_empty() {
        return Err("No certificates found in cert file".to_string());
    }
    let key = PrivateKeyDer::from_pem_reader(std::io::BufReader::new(key_file)).map_err(|e| format!("Failed to parse key: {e}"))?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("Failed to build TLS config: {e}"))
}
