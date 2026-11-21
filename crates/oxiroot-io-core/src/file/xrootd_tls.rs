//! TLS and token authentication for the `XRootD` client (the `xrootd-tls`
//! feature).
//!
//! A client that can do TLS says so in its `kXR_protocol` request
//! (`kXR_ableTLS`, plus `kXR_wantTLS` for a `roots://` URL); a server that wants
//! TLS answers `kXR_gotoTLS`, and both switch the connection to TLS before the
//! login. Token authentication (the `ztn` security protocol) is only allowed
//! over TLS: it sends a WLCG bearer token, found the way the
//! [WLCG bearer token discovery] spec says, in one `kXR_auth` request.
//!
//! Server certificates are checked against the Mozilla root store plus the
//! grid CAs in `$X509_CERT_DIR` (default `/etc/grid-security/certificates`),
//! where the IGTF CAs that sign most grid hosts live.
//!
//! [WLCG bearer token discovery]: https://doi.org/10.5281/zenodo.3937438

use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::error::{Error, Result};

/// Where grid CA certificates live when `$X509_CERT_DIR` is not set.
const GRID_CA_DIR: &str = "/etc/grid-security/certificates";

/// Options for opening a file over `XRootD`: the bearer token and the CAs to
/// trust (the `xrootd-tls` feature).
///
/// The defaults find both on their own: the token from the environment the
/// WLCG way (`BEARER_TOKEN`, `BEARER_TOKEN_FILE`, `$XDG_RUNTIME_DIR/bt_u<uid>`,
/// `/tmp/bt_u<uid>`), and the CAs from the Mozilla root store plus
/// `$X509_CERT_DIR` (default `/etc/grid-security/certificates`).
///
/// ```no_run
/// use oxiroot_io_core::{FileReader, XrootdOptions, XrootdSource};
///
/// let opts = XrootdOptions::new().token(std::fs::read_to_string("token.jwt")?);
/// let source = XrootdSource::open_with("roots://eos.example.org//data/run.root", &opts)?;
/// let file = FileReader::from_source(Box::new(source))?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct XrootdOptions {
    token: Option<String>,
    ca_dir: Option<PathBuf>,
}

impl XrootdOptions {
    /// The defaults: token and CAs found from the environment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Authenticate with this bearer token (surrounding whitespace is ignored)
    /// instead of looking for one in the environment.
    #[must_use]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }

    /// Trust the CA certificates (PEM) in this directory, besides the Mozilla
    /// root store, instead of `$X509_CERT_DIR`.
    #[must_use]
    pub fn ca_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.ca_dir = Some(dir.into());
        self
    }

    /// The token to send: the one given, or the first found in the
    /// environment. `Ok(None)` when there is none.
    pub(super) fn bearer_token(&self) -> Result<Option<String>> {
        match &self.token {
            Some(t) => Ok(strip(t).map(str::to_string)),
            None => discover_token(),
        }
    }

    fn ca_dir_or_default(&self) -> Option<PathBuf> {
        self.ca_dir.clone().or_else(|| {
            std::env::var_os("X509_CERT_DIR")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
                .or_else(|| Some(PathBuf::from(GRID_CA_DIR)))
        })
    }
}

impl fmt::Debug for XrootdOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the token itself.
        f.debug_struct("XrootdOptions")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("ca_dir", &self.ca_dir)
            .finish()
    }
}

/// The token with surrounding whitespace removed, or `None` if that leaves
/// nothing.
fn strip(token: &str) -> Option<&str> {
    Some(token.trim()).filter(|t| !t.is_empty())
}

/// Find a bearer token the WLCG way: the `BEARER_TOKEN` variable, the file
/// `BEARER_TOKEN_FILE` names, `$XDG_RUNTIME_DIR/bt_u<uid>`, then
/// `/tmp/bt_u<uid>`. The first that holds a token wins.
fn discover_token() -> Result<Option<String>> {
    if let Some(t) = std::env::var("BEARER_TOKEN")
        .ok()
        .as_deref()
        .and_then(strip)
    {
        return Ok(Some(t.to_string()));
    }
    let mut files = Vec::new();
    if let Some(p) = std::env::var_os("BEARER_TOKEN_FILE").filter(|p| !p.is_empty()) {
        files.push(PathBuf::from(p));
    }
    if let Some(uid) = euid() {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
            files.push(Path::new(&dir).join(format!("bt_u{uid}")));
        }
        files.push(PathBuf::from(format!("/tmp/bt_u{uid}")));
    }
    for path in files {
        if let Some(t) = read_token_file(&path)? {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

#[cfg(unix)]
fn euid() -> Option<u32> {
    Some(rustix::process::geteuid().as_raw())
}

#[cfg(not(unix))]
fn euid() -> Option<u32> {
    None
}

/// Read a token file: `Ok(None)` if it does not exist or is blank. Like the
/// `XRootD` client, refuse a file that others can read.
fn read_token_file(path: &Path) -> Result<Option<String>> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(token_err(path, &e.to_string())),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(token_err(
                path,
                "the file is readable by others (it must be mode 0600 or stricter)",
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    let text = std::fs::read_to_string(path).map_err(|e| token_err(path, &e.to_string()))?;
    Ok(strip(&text).map(str::to_string))
}

fn token_err(path: &Path, why: &str) -> Error {
    Error::InvalidInput(format!("bearer token {}: {why}", path.display()))
}

/// The `ztn` credential for `token`: an 8-byte header (`"ztn\0"`, version 0,
/// `'T'` for "token", two reserved bytes), the token's length with its null
/// byte as a big-endian `u16`, then the token and a null byte.
///
/// `parms` is what the server sent with `&P=ztn,`: `<options+version>:<max
/// token size>:`. A token that does not fit is an error.
pub(super) fn ztn_credential(token: &str, parms: &str) -> Result<Vec<u8>> {
    let max = parms
        .split(':')
        .nth(1)
        .and_then(|m| m.parse::<usize>().ok())
        .filter(|&m| m > 0)
        .ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("root://: malformed ztn parameters from the server: {parms:?}"),
            ))
        })?;
    if token.len() >= max {
        return Err(Error::InvalidInput(format!(
            "bearer token of {} bytes is too big for the server (at most {})",
            token.len(),
            max - 1
        )));
    }
    let len = u16::try_from(token.len() + 1).map_err(|_| {
        Error::InvalidInput(format!("bearer token of {} bytes is too big", token.len()))
    })?;
    let mut cred = Vec::with_capacity(11 + token.len());
    cred.extend_from_slice(b"ztn\0");
    cred.extend_from_slice(&[0, b'T', 0, 0]);
    cred.extend_from_slice(&len.to_be_bytes());
    cred.extend_from_slice(token.as_bytes());
    cred.push(0);
    Ok(cred)
}

/// A TLS client stream over the `XRootD` connection.
pub(super) type TlsStream = StreamOwned<ClientConnection, TcpStream>;

/// Switch `tcp` to TLS with `host`, verifying its certificate, and finish the
/// handshake before returning.
pub(super) fn connect(tcp: TcpStream, host: &str, opts: &XrootdOptions) -> Result<TlsStream> {
    let config = client_config(opts)?;
    let name = ServerName::try_from(host.to_string())
        .map_err(|e| Error::InvalidInput(format!("roots://: bad host name {host:?}: {e}")))?;
    let conn = ClientConnection::new(config, name).map_err(|e| tls_err(host, &e.to_string()))?;
    let mut stream = StreamOwned::new(conn, tcp);
    while stream.conn.is_handshaking() {
        stream
            .conn
            .complete_io(&mut stream.sock)
            .map_err(|e| tls_err(host, &e.to_string()))?;
    }
    Ok(stream)
}

fn tls_err(host: &str, why: &str) -> Error {
    Error::Io(std::io::Error::other(format!(
        "root://{host}: TLS handshake: {why}"
    )))
}

fn client_config(opts: &XrootdOptions) -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(dir) = opts.ca_dir_or_default() {
        add_ca_dir(&mut roots, &dir);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Io(std::io::Error::other(format!("TLS setup: {e}"))))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Add every PEM certificate in `dir` (an OpenSSL hashed CA directory, such as
/// `/etc/grid-security/certificates`) to `roots`. Files that hold no
/// certificate — signing policies, CRL URLs, namespaces — are skipped, and so
/// is a directory that does not exist.
fn add_ca_dir(roots: &mut RootCertStore, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(certs) = CertificateDer::pem_file_iter(&path) else {
            continue;
        };
        let certs: Vec<_> = certs.filter_map(std::result::Result::ok).collect();
        roots.add_parsable_certificates(certs);
    }
}

/// The connection to an `XRootD` server: plain TCP, or TLS on top of it.
pub(super) enum Stream {
    Plain(TcpStream),
    Tls(Box<TlsStream>),
}

impl Stream {
    pub(super) fn is_tls(&self) -> bool {
        matches!(self, Self::Tls(_))
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ztn_credential_layout() {
        let cred = ztn_credential("abc", "0:4096:").unwrap();
        assert_eq!(
            cred,
            [b"ztn\0".as_slice(), &[0, b'T', 0, 0], &[0, 4], b"abc\0"].concat()
        );
    }

    #[test]
    fn ztn_credential_checks_the_size_and_parameters() {
        assert!(
            ztn_credential("abcd", "0:4:").is_err(),
            "token + null > max"
        );
        assert!(ztn_credential("abc", "0:4:").is_ok());
        assert!(ztn_credential("abc", "garbage").is_err());
        assert!(ztn_credential("abc", "0:0:").is_err());
    }

    #[test]
    fn explicit_tokens_are_trimmed_and_blank_ones_ignored() {
        let opts = XrootdOptions::new().token("  tok\n");
        assert_eq!(opts.bearer_token().unwrap().as_deref(), Some("tok"));
        assert_eq!(strip(" \n\t"), None);
    }

    #[test]
    fn debug_redacts_the_token() {
        let s = format!("{:?}", XrootdOptions::new().token("secret"));
        assert!(!s.contains("secret"), "{s}");
    }

    #[cfg(unix)]
    #[test]
    fn token_files_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("oxiroot-bt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token");
        std::fs::write(&path, "tok\n").unwrap();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_token_file(&path).unwrap().as_deref(), Some("tok"));

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = read_token_file(&path).unwrap_err();
        assert!(err.to_string().contains("readable by others"), "{err}");

        assert_eq!(read_token_file(&dir.join("missing")).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
