//! An `XRootD` (`root://`) byte-range [`ByteSource`] (the `xrootd` feature).
//!
//! `XRootD` is the binary TCP protocol most CERN data is served over. This is a
//! read-only client: it speaks the `xroot` protocol directly (no libXrdCl, and
//! just `std::net` by default) and authenticates with the credential-free
//! `unix` security protocol, so it reads world-readable files from servers that
//! offer `unix` auth (e.g. `root://eospublic.cern.ch`).
//!
//! The `xrootd-tls` feature adds TLS — `roots://` URLs and servers that require
//! it — and token authentication (the `ztn` protocol) with a WLCG bearer token,
//! for access-controlled data; see `XrootdOptions`. GSI (X.509 proxy)
//! and Kerberos authentication are not supported.
//!
//! `kXR_read(handle, offset, length)` maps directly onto
//! [`ByteSource::read_at`], so an [`XrootdSource`] plugs into [`FileReader`] exactly
//! like the HTTP source, and every reader fetches only the ranges it touches.
//! A batch of ranges — a branch's baskets, a column's pages — goes out as one
//! `kXR_readv` (vector read), one round trip for up to 1024 ranges.
//!
//! The wire framing here was validated against `root://eospublic.cern.ch`.
//!
//! [`FileReader`]: super::reader::FileReader

use crate::cast::SaturatingFrom;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;

use super::source::ByteSource;
#[cfg(feature = "xrootd-tls")]
use super::xrootd_tls::{self, Stream, XrootdOptions};
use crate::error::{Error, Result};

/// Without TLS the connection is the TCP stream itself.
#[cfg(not(feature = "xrootd-tls"))]
type Stream = TcpStream;

/// Without TLS there is nothing to configure: no token, no CAs.
#[cfg(not(feature = "xrootd-tls"))]
#[derive(Default)]
struct XrootdOptions {}

// Request opcodes (`kXR_*`).
const KXR_AUTH: u16 = 3000;
const KXR_CLOSE: u16 = 3003;
const KXR_LOGIN: u16 = 3007;
const KXR_OPEN: u16 = 3010;
#[cfg(feature = "xrootd-tls")]
const KXR_PROTOCOL: u16 = 3006;
const KXR_READ: u16 = 3013;
const KXR_READV: u16 = 3025;
const KXR_STAT: u16 = 3017;

// Response status codes.
const KXR_OK: u16 = 0;
const KXR_OKSOFAR: u16 = 4000;
const KXR_ERROR: u16 = 4003;
const KXR_REDIRECT: u16 = 4004;
const KXR_WAIT: u16 = 4005;

/// The protocol version this client speaks (`kXR_PROTOCOLVERSION`, 5.2.0).
#[cfg(feature = "xrootd-tls")]
const KXR_PROTOCOL_VERSION: i32 = 0x0000_0520;
/// `kXR_protocol` request flags: the client can do TLS, and wants it now.
#[cfg(feature = "xrootd-tls")]
const KXR_ABLE_TLS: u8 = 0x02;
#[cfg(feature = "xrootd-tls")]
const KXR_WANT_TLS: u8 = 0x04;
/// `kXR_protocol` expectation: a login follows.
#[cfg(feature = "xrootd-tls")]
const KXR_EXP_LOGIN: u8 = 0x03;
/// `kXR_protocol` response flags: the server can do TLS, and wants to now.
#[cfg(feature = "xrootd-tls")]
const KXR_HAVE_TLS: u32 = 0x8000_0000;
#[cfg(feature = "xrootd-tls")]
const KXR_GOTO_TLS: u32 = 0x4000_0000;

/// How many `kXR_wait` replies an open honours before giving up.
const MAX_WAITS: usize = 30;
/// The largest payload a control request (login, auth, open, stat, close) may
/// return; reads are capped at the length they asked for.
const SMALL_RESPONSE: usize = 64 * 1024;

// Open option: read-only.
const KXR_OPEN_READ: u16 = 0x10;

/// The most elements one `kXR_readv` may carry (`XrdProto::maxRvecsz`).
const READV_MAX_ELEMENTS: usize = 1024;
/// The most bytes one `kXR_readv` element may ask for (`XrdProto::maxRvecln`);
/// a longer range is split across elements.
const READV_MAX_ELEMENT_LEN: usize = 2_097_136;
/// The size of a `readahead_list` element: fhandle[4], rlen(i32), offset(i64).
const READV_ELEMENT: usize = 16;

/// Default `XRootD` port.
const DEFAULT_PORT: u16 = 1094;
/// Guard against a redirect loop.
const MAX_REDIRECTS: u32 = 8;
/// Connect / read timeout.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A parsed `root[s]://[user@]host[:port]//path` URL.
struct XrootdUrl {
    host: String,
    port: u16,
    path: String,
    /// A `roots://` URL: the whole session must be over TLS.
    tls: bool,
}

impl XrootdUrl {
    fn parse(url: &str) -> Result<Self> {
        let (rest, tls) = if let Some(rest) = url.strip_prefix("root://") {
            (rest, false)
        } else if let Some(rest) = url.strip_prefix("roots://") {
            (rest, true)
        } else {
            return Err(Error::InvalidInput(format!("not a root:// URL: {url:?}")));
        };
        if tls && cfg!(not(feature = "xrootd-tls")) {
            return Err(Error::Unsupported(format!(
                "roots:// needs TLS, which needs the `xrootd-tls` feature: {url:?}"
            )));
        }
        // Authority is up to the first '/', the path is the remainder. XRootD's
        // canonical form uses a double slash — `root://host//abs/path` — so the
        // remainder already carries the leading slash of the absolute path.
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let authority = authority.rsplit('@').next().unwrap_or(authority); // drop any user@
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(DEFAULT_PORT)),
            None => (authority.to_string(), DEFAULT_PORT),
        };
        if host.is_empty() {
            return Err(Error::InvalidInput(format!(
                "root:// URL has no host: {url:?}"
            )));
        }
        let path = if path.is_empty() {
            return Err(Error::InvalidInput(format!(
                "root:// URL has no path: {url:?}"
            )));
        } else if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        Ok(Self {
            host,
            port,
            path,
            tls,
        })
    }
}

/// A live connection to one `XRootD` server with a file open, serialized so the
/// request/response stream stays in step under concurrent reads.
struct Conn {
    stream: Stream,
    stream_id: u16,
    fhandle: [u8; 4],
}

impl Conn {
    /// Handshake, TLS if either side wants it, login and authentication
    /// against `host:port`, then open `path` for reading, following redirects.
    /// With `tls` (a `roots://` URL) every server on the way must do TLS.
    /// Returns the connection and the file size.
    fn open(
        host: &str,
        port: u16,
        path: &str,
        tls: bool,
        opts: &XrootdOptions,
    ) -> Result<(Self, u64)> {
        let mut host = host.to_string();
        let mut port = port;
        // A redirector (e.g. EOS) hands back an opaque capability token that must
        // be appended to the path when the data server re-opens the file.
        let mut opaque = String::new();
        for _ in 0..MAX_REDIRECTS {
            let mut stream = TcpStream::connect((host.as_str(), port))
                .map_err(|e| io_err(&format!("connecting to {host}:{port}"), e))?;
            stream.set_read_timeout(Some(TIMEOUT)).ok();
            stream.set_write_timeout(Some(TIMEOUT)).ok();

            handshake(&mut stream)?;
            let mut conn = Self {
                #[cfg(feature = "xrootd-tls")]
                stream: Stream::Plain(stream),
                #[cfg(not(feature = "xrootd-tls"))]
                stream,
                stream_id: 0,
                fhandle: [0; 4],
            };
            #[cfg(feature = "xrootd-tls")]
            conn.negotiate_tls(&host, tls, opts)?;
            #[cfg(not(feature = "xrootd-tls"))]
            let _ = tls; // `roots://` is refused at parse time
            conn.login(opts)?;

            let open_path = if opaque.is_empty() {
                path.to_string()
            } else {
                format!("{path}?{opaque}")
            };
            match conn.try_open(&open_path)? {
                OpenOutcome::Opened => {
                    let size = conn.fstat()?;
                    return Ok((conn, size));
                }
                OpenOutcome::Redirect {
                    host: h,
                    port: p,
                    opaque: o,
                } => {
                    host = h;
                    port = p;
                    opaque = o;
                }
            }
        }
        Err(Error::Io {
            kind: std::io::ErrorKind::Other,
            message: format!("root://{host}: too many redirects opening {path:?}"),
        })
    }

    /// Send a request (24-byte header + `data`) and return the collected
    /// response payload, resolving `kXR_wait` (retry) and surfacing errors.
    ///
    /// `max_len` is the most response payload this request can legitimately
    /// carry; a server that announces more is a protocol error, so it cannot
    /// make the client buffer an arbitrary amount.
    fn request(
        &mut self,
        reqid: u16,
        body: [u8; 16],
        data: &[u8],
        max_len: usize,
    ) -> Result<(u16, Vec<u8>)> {
        self.stream_id = self.stream_id.wrapping_add(1);
        let mut hdr = Vec::with_capacity(24 + data.len());
        hdr.extend_from_slice(&self.stream_id.to_be_bytes());
        hdr.extend_from_slice(&reqid.to_be_bytes());
        hdr.extend_from_slice(&body);
        hdr.extend_from_slice(&i32::saturating_from(data.len()).to_be_bytes());
        hdr.extend_from_slice(data);
        self.stream
            .write_all(&hdr)
            .and_then(|()| self.stream.flush())
            .map_err(|e| io_err("sending request", e))?;
        self.read_response(max_len)
    }

    /// Read one logical response, concatenating `kXR_oksofar` continuations,
    /// of at most `max_len` payload bytes in all. Each frame is read as its
    /// bytes arrive rather than into a buffer sized by the frame header, so a
    /// header announcing more than the server sends cannot force a large
    /// allocation.
    fn read_response(&mut self, max_len: usize) -> Result<(u16, Vec<u8>)> {
        let mut payload = Vec::new();
        loop {
            let mut head = [0u8; 8];
            self.read_exact(&mut head)?;
            let status = u16::from_be_bytes([head[2], head[3]]);
            let dlen = usize::saturating_from(
                i32::from_be_bytes([head[4], head[5], head[6], head[7]]).max(0),
            );
            if payload.len() + dlen > max_len {
                return Err(protocol_err(format!(
                    "root://: response of {} bytes exceeds the {max_len} this request allows",
                    payload.len() + dlen
                )));
            }
            let start = payload.len();
            (&mut self.stream)
                .take(dlen as u64)
                .read_to_end(&mut payload)
                .map_err(|e| io_err("reading response", e))?;
            if payload.len() - start != dlen {
                return Err(io_err(
                    "reading response",
                    std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
                ));
            }
            if status != KXR_OKSOFAR {
                return Ok((status, payload));
            }
        }
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.stream
            .read_exact(buf)
            .map_err(|e| io_err("reading response", e))
    }

    /// Say that this client can do TLS (`kXR_protocol` with `kXR_ableTLS`,
    /// and `kXR_wantTLS` when `require`d) and switch the connection to TLS if
    /// the server asks for it, or if `require`d. A server that cannot do TLS
    /// fails a required switch.
    #[cfg(feature = "xrootd-tls")]
    fn negotiate_tls(&mut self, host: &str, require: bool, opts: &XrootdOptions) -> Result<()> {
        let mut body = [0u8; 16];
        body[0..4].copy_from_slice(&KXR_PROTOCOL_VERSION.to_be_bytes());
        body[4] = KXR_ABLE_TLS | if require { KXR_WANT_TLS } else { 0 };
        body[5] = KXR_EXP_LOGIN;
        let (status, payload) = self.request(KXR_PROTOCOL, body, &[], SMALL_RESPONSE)?;
        if status != KXR_OK {
            return Err(server_error("protocol", status, &payload));
        }
        // Body: pval(int32), flags(int32), then optional security requirements.
        // An older server may send less: no flags, so no TLS.
        let flags = payload
            .get(4..8)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        if require && flags & KXR_HAVE_TLS == 0 {
            return Err(Error::Unsupported(format!(
                "roots://{host}: the server does not offer TLS"
            )));
        }
        if flags & KXR_GOTO_TLS != 0 || require {
            self.start_tls(host, opts)?;
        }
        Ok(())
    }

    /// Switch the connection to TLS: from here on every byte is encrypted.
    #[cfg(feature = "xrootd-tls")]
    fn start_tls(&mut self, host: &str, opts: &XrootdOptions) -> Result<()> {
        let Stream::Plain(tcp) = &self.stream else {
            return Ok(());
        };
        let tcp = tcp.try_clone().map_err(|e| io_err("starting TLS", e))?;
        self.stream = Stream::Tls(Box::new(xrootd_tls::connect(tcp, host, opts)?));
        Ok(())
    }

    fn login(&mut self, opts: &XrootdOptions) -> Result<()> {
        // Body: pid(0), username[8], reserved, ability, capver, role. capver is
        // the protocol version (4) *without* the async-capability bit (0x80), so
        // the server always answers reads synchronously — this client does not
        // implement the async `kXR_waitresp`/`kXR_attn` path.
        let mut body = [0u8; 16];
        body[4..10].copy_from_slice(b"nobody");
        body[14] = 0x04;
        let (status, payload) = self.request(KXR_LOGIN, body, &[], SMALL_RESPONSE)?;
        if status != KXR_OK {
            return Err(server_error("login", status, &payload));
        }
        // Payload: 16-byte session id, then (if the server requires auth) a
        // security spec such as "&P=gsi,…&P=ztn,0:4096:&P=unix".
        if payload.len() > 16 {
            self.authenticate(&payload[16..], opts)?;
        }
        Ok(())
    }

    /// Authenticate with the first protocol in the server's `spec` (its order
    /// of preference) that this client can do: `ztn` with a bearer token over
    /// TLS (the `xrootd-tls` feature), or the credential-free `unix`.
    fn authenticate(&mut self, spec: &[u8], opts: &XrootdOptions) -> Result<()> {
        let spec = String::from_utf8_lossy(spec);
        let spec = spec.trim_end_matches('\0');
        let mut skipped: Vec<&str> = Vec::new();
        for entry in spec.split('&').filter_map(|e| e.strip_prefix("P=")) {
            let (proto, parms) = entry.split_once(',').unwrap_or((entry, ""));
            match proto {
                "unix" => return self.auth_unix(),
                "ztn" => {
                    #[cfg(feature = "xrootd-tls")]
                    {
                        if !self.stream.is_tls() {
                            skipped
                                .push("`ztn` (token) needs TLS, which the server did not turn on");
                        } else if let Some(token) = opts.bearer_token()? {
                            return self.auth_ztn(&token, parms);
                        } else {
                            skipped.push(
                                "`ztn` (token): no bearer token found in BEARER_TOKEN, \
                                 BEARER_TOKEN_FILE, $XDG_RUNTIME_DIR/bt_u<uid> or /tmp/bt_u<uid>",
                            );
                        }
                    }
                    #[cfg(not(feature = "xrootd-tls"))]
                    {
                        let _ = (parms, opts);
                        skipped.push("`ztn` (token) needs the `xrootd-tls` feature");
                    }
                }
                "gsi" => skipped.push("`gsi` (X.509 proxy) is not supported"),
                _ => {}
            }
        }
        let mut msg = format!(
            "root://: the server requires authentication this client cannot do; offered: {spec}"
        );
        for why in skipped {
            msg.push_str("; ");
            msg.push_str(why);
        }
        Err(Error::Unsupported(msg))
    }

    /// Token authentication: one `kXR_auth` carrying the `ztn` credential.
    #[cfg(feature = "xrootd-tls")]
    fn auth_ztn(&mut self, token: &str, parms: &str) -> Result<()> {
        let cred = xrootd_tls::ztn_credential(token, parms)?;
        let mut body = [0u8; 16];
        body[12..16].copy_from_slice(b"ztn\0");
        let (status, payload) = self.request(KXR_AUTH, body, &cred, SMALL_RESPONSE)?;
        if status != KXR_OK {
            return Err(server_error("token (ztn) authentication", status, &payload));
        }
        Ok(())
    }

    fn auth_unix(&mut self) -> Result<()> {
        // Body: reserved[12] + credtype[4]="unix"; credential is the tag itself.
        let mut body = [0u8; 16];
        body[12..16].copy_from_slice(b"unix");
        let (status, payload) = self.request(KXR_AUTH, body, b"unix\0", SMALL_RESPONSE)?;
        if status != KXR_OK {
            return Err(server_error("unix authentication", status, &payload));
        }
        Ok(())
    }

    fn try_open(&mut self, path: &str) -> Result<OpenOutcome> {
        // A server may ask us to wait and retry; give up after a bounded
        // number of waits instead of retrying (or recursing) forever.
        for _ in 0..MAX_WAITS {
            if let Some(outcome) = self.open_once(path)? {
                return Ok(outcome);
            }
        }
        Err(protocol_err(format!(
            "root://: server asked to wait {MAX_WAITS} times while opening {path}"
        )))
    }

    /// One `kXR_open` attempt: the outcome, or `None` after honouring a
    /// `kXR_wait`.
    fn open_once(&mut self, path: &str) -> Result<Option<OpenOutcome>> {
        // Body: mode(0) + options + reserved[12]; data = path.
        let mut body = [0u8; 16];
        body[0..2].copy_from_slice(&0u16.to_be_bytes()); // mode
        body[2..4].copy_from_slice(&KXR_OPEN_READ.to_be_bytes());
        let (status, payload) = self.request(KXR_OPEN, body, path.as_bytes(), SMALL_RESPONSE)?;
        match status {
            KXR_OK => {
                if payload.len() < 4 {
                    return Err(protocol_err("root://: short open response".into()));
                }
                self.fhandle.copy_from_slice(&payload[0..4]);
                Ok(Some(OpenOutcome::Opened))
            }
            KXR_REDIRECT => {
                let (host, port, opaque) = parse_redirect(&payload)?;
                Ok(Some(OpenOutcome::Redirect { host, port, opaque }))
            }
            KXR_WAIT => {
                let secs = payload
                    .get(0..4)
                    .map_or(1, |b| {
                        u64::saturating_from(i32::from_be_bytes([b[0], b[1], b[2], b[3]]).max(0))
                    })
                    .min(10);
                std::thread::sleep(Duration::from_secs(secs));
                Ok(None)
            }
            _ => Err(server_error(&format!("opening {path}"), status, &payload)),
        }
    }

    /// Stat the open file by its handle (`fstat`) and return its size. The
    /// `kXR_stat` response is `"id size flags modtime"`.
    fn fstat(&mut self) -> Result<u64> {
        // Body: reserved[11] + options(0) + fhandle[4]; no path (dlen 0) => fstat.
        let mut body = [0u8; 16];
        body[12..16].copy_from_slice(&self.fhandle);
        let (status, payload) = self.request(KXR_STAT, body, &[], SMALL_RESPONSE)?;
        if status != KXR_OK {
            return Err(server_error("stat", status, &payload));
        }
        let s = String::from_utf8_lossy(&payload);
        s.split_whitespace()
            .nth(1)
            .and_then(|f| f.parse().ok())
            .ok_or_else(|| protocol_err(format!("root://: unparseable stat {:?}", s.trim())))
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Bytes> {
        // Body: fhandle[4] + offset(i64) + rlen(i32).
        let mut body = [0u8; 16];
        body[0..4].copy_from_slice(&self.fhandle);
        body[4..12].copy_from_slice(&(offset).cast_signed().to_be_bytes());
        body[12..16].copy_from_slice(&i32::saturating_from(len).to_be_bytes());
        let (status, payload) = self.request(KXR_READ, body, &[], len)?;
        if status != KXR_OK {
            return Err(server_error("reading", status, &payload));
        }
        if payload.len() != len {
            return Err(Error::UnexpectedEof {
                needed: len,
                available: payload.len(),
            });
        }
        Ok(Bytes::from(payload))
    }

    /// Read several ranges in one `kXR_readv` round trip per
    /// [`READV_MAX_ELEMENTS`] elements, splitting ranges longer than an element
    /// may be. Each element of the reply is checked against what was asked.
    fn read_many(&mut self, ranges: &[(u64, usize)]) -> Result<Vec<Bytes>> {
        // The elements to ask for: (range index, offset, length).
        let mut elements = Vec::new();
        for (i, &(offset, len)) in ranges.iter().enumerate() {
            let mut done = 0usize;
            while done < len {
                let n = (len - done).min(READV_MAX_ELEMENT_LEN);
                elements.push((i, offset + done as u64, n));
                done += n;
            }
        }
        let mut out: Vec<Vec<u8>> = ranges.iter().map(|&(_, l)| Vec::with_capacity(l)).collect();
        for chunk in elements.chunks(READV_MAX_ELEMENTS) {
            let mut list = Vec::with_capacity(chunk.len() * READV_ELEMENT);
            for &(_, offset, len) in chunk {
                list.extend_from_slice(&self.fhandle);
                list.extend_from_slice(&i32::saturating_from(len).to_be_bytes());
                list.extend_from_slice(&(offset).cast_signed().to_be_bytes());
            }
            let expected: usize = chunk.iter().map(|&(_, _, l)| READV_ELEMENT + l).sum();
            let (status, payload) = self.request(KXR_READV, [0u8; 16], &list, expected)?;
            if status != KXR_OK {
                return Err(server_error("vector read", status, &payload));
            }
            let mut at = 0usize;
            for &(i, offset, len) in chunk {
                let head = payload.get(at..at + READV_ELEMENT).ok_or_else(|| {
                    protocol_err("root://: vector read reply ends before its elements".into())
                })?;
                let rlen = i32::from_be_bytes([head[4], head[5], head[6], head[7]]);
                let roff = i64::from_be_bytes(head[8..16].try_into().expect("8 bytes"));
                if rlen < 0 || usize::saturating_from(rlen) != len || roff != offset.cast_signed() {
                    return Err(protocol_err(format!(
                        "root://: vector read returned {rlen} bytes at {roff} for {len} at {offset}"
                    )));
                }
                at += READV_ELEMENT;
                let data = payload
                    .get(at..at + len)
                    .ok_or_else(|| Error::UnexpectedEof {
                        needed: len,
                        available: payload.len().saturating_sub(at),
                    })?;
                out[i].extend_from_slice(data);
                at += len;
            }
            if at != payload.len() {
                return Err(protocol_err(format!(
                    "root://: vector read reply has {} bytes past its elements",
                    payload.len() - at
                )));
            }
        }
        Ok(out.into_iter().map(Bytes::from).collect())
    }

    fn close(&mut self) {
        let mut body = [0u8; 16];
        body[0..4].copy_from_slice(&self.fhandle);
        let _ = self.request(KXR_CLOSE, body, &[], SMALL_RESPONSE);
    }
}

enum OpenOutcome {
    Opened,
    Redirect {
        host: String,
        port: u16,
        opaque: String,
    },
}

/// A ROOT file accessed over `XRootD` (`root://`) with byte-range reads.
pub struct XrootdSource {
    conn: Mutex<Conn>,
    size: u64,
    url: String,
}

impl std::fmt::Debug for XrootdSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XrootdSource")
            .field("url", &self.url)
            .field("size", &self.size)
            .finish()
    }
}

impl XrootdSource {
    /// Connect to `url` (`root://host[:port]//path`, or `roots://` for TLS
    /// with the `xrootd-tls` feature), log in, authenticate, and open the file
    /// for reading. A bearer token and grid CAs are found from the environment
    /// (see `XrootdOptions`).
    pub fn open(url: &str) -> Result<Self> {
        Self::connect(url, &XrootdOptions::default())
    }

    /// [`open`](Self::open) with an explicit bearer token or CA directory.
    #[cfg(feature = "xrootd-tls")]
    pub fn open_with(url: &str, opts: &XrootdOptions) -> Result<Self> {
        Self::connect(url, opts)
    }

    fn connect(url: &str, opts: &XrootdOptions) -> Result<Self> {
        let parsed = XrootdUrl::parse(url)?;
        let (conn, size) = Conn::open(&parsed.host, parsed.port, &parsed.path, parsed.tls, opts)?;
        Ok(Self {
            conn: Mutex::new(conn),
            size,
            url: url.to_string(),
        })
    }
}

impl ByteSource for XrootdSource {
    fn len(&self) -> u64 {
        self.size
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Bytes> {
        if len == 0 {
            return Ok(Bytes::new());
        }
        offset
            .checked_add(len as u64)
            .filter(|&e| e <= self.size)
            .ok_or_else(|| Error::UnexpectedEof {
                needed: len,
                available: usize::saturating_from(self.size.saturating_sub(offset)),
            })?;
        // One request/response at a time keeps the shared stream in step.
        let mut conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.read_at(offset, len)
    }

    fn read_many(&self, ranges: &[(u64, usize)]) -> Result<Vec<Bytes>> {
        for &(offset, len) in ranges {
            offset
                .checked_add(len as u64)
                .filter(|&e| e <= self.size)
                .ok_or_else(|| Error::UnexpectedEof {
                    needed: len,
                    available: usize::saturating_from(self.size.saturating_sub(offset)),
                })?;
        }
        let mut conn = self
            .conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        conn.read_many(ranges)
    }

    fn batches_reads(&self) -> bool {
        true
    }
}

impl Drop for XrootdSource {
    fn drop(&mut self) {
        if let Ok(mut conn) = self.conn.lock() {
            conn.close();
        }
    }
}

/// Send the initial 20-byte client handshake and consume the 16-byte reply.
fn handshake(stream: &mut TcpStream) -> Result<()> {
    // Five big-endian int32: 0, 0, 0, 4, 2012.
    let mut req = [0u8; 20];
    req[12..16].copy_from_slice(&4i32.to_be_bytes());
    req[16..20].copy_from_slice(&2012i32.to_be_bytes());
    stream
        .write_all(&req)
        .map_err(|e| io_err("sending handshake", e))?;
    let mut reply = [0u8; 16];
    stream
        .read_exact(&mut reply)
        .map_err(|e| io_err("reading handshake reply", e))?;
    Ok(())
}

/// Parse a `kXR_redirect` payload: `int32 port` then `host[:port][?opaque]`.
/// Returns the host, the port (the leading int32), and the opaque capability
/// token (the `?…` tail, empty if none) to forward on the data server's re-open.
fn parse_redirect(payload: &[u8]) -> Result<(String, u16, String)> {
    if payload.len() < 4 {
        return Err(protocol_err("root://: short redirect response".into()));
    }
    let port = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let rest = String::from_utf8_lossy(&payload[4..]);
    let (hostport, opaque) = rest.split_once('?').unwrap_or((rest.as_ref(), ""));
    let host = hostport.split(':').next().unwrap_or("").trim().to_string();
    if host.is_empty() || !(0..=65535).contains(&port) {
        return Err(protocol_err(format!(
            "root://: bad redirect to {rest:?}:{port}"
        )));
    }
    Ok((host, u16::saturating_from(port), opaque.to_string()))
}

/// Turn a `kXR_error`/unexpected status into an [`Error`]. The error payload is
/// `int32 errnum` then a message string.
fn server_error(ctx: &str, status: u16, payload: &[u8]) -> Error {
    if status == KXR_ERROR && payload.len() >= 4 {
        let msg = String::from_utf8_lossy(&payload[4..]);
        let msg = msg.trim_end_matches('\0');
        Error::Io {
            kind: std::io::ErrorKind::Other,
            message: format!("root:// {ctx}: {msg}"),
        }
    } else {
        protocol_err(format!("root:// {ctx}: unexpected status {status}"))
    }
}

/// A server response that breaks the `XRootD` protocol.
fn protocol_err(message: String) -> Error {
    Error::Io {
        kind: std::io::ErrorKind::InvalidData,
        message,
    }
}

fn io_err(ctx: &str, e: std::io::Error) -> Error {
    Error::Io {
        kind: e.kind(),
        message: format!("{ctx}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_root_urls() {
        let u = XrootdUrl::parse("root://eospublic.cern.ch//eos/root-eos/hsimple.root").unwrap();
        assert_eq!(u.host, "eospublic.cern.ch");
        assert_eq!(u.port, DEFAULT_PORT);
        assert_eq!(u.path, "/eos/root-eos/hsimple.root");

        let u = XrootdUrl::parse("root://user@host.example:1095//a/b.root").unwrap();
        assert_eq!(u.host, "host.example");
        assert_eq!(u.port, 1095);
        assert_eq!(u.path, "/a/b.root");

        assert!(XrootdUrl::parse("https://x/y").is_err());
        assert!(XrootdUrl::parse("root://host").is_err()); // no path
    }

    #[test]
    fn parses_a_redirect_payload() {
        let mut p = 1095i32.to_be_bytes().to_vec();
        p.extend_from_slice(b"dataserver.cern.ch?authz=abc&xrd.k=v");
        let (h, port, opaque) = parse_redirect(&p).unwrap();
        assert_eq!(h, "dataserver.cern.ch");
        assert_eq!(port, 1095);
        assert_eq!(opaque, "authz=abc&xrd.k=v");
    }
}
