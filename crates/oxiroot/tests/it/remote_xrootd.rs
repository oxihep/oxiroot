//! Remote reads over the `XRootD` `root://` protocol (`xrootd` feature).
//!
//! A tiny in-process `XRootD` server speaks enough of the binary protocol —
//! handshake, protocol (with a switch to TLS), login, `unix` and `ztn` (token)
//! auth, open, fstat, read, vector read, close — to serve a fixture
//! `.root` file, so the client is tested hermetically (no network). The wire
//! framing mirrors what was verified against `root://eospublic.cern.ch` — CERN's
//! anonymous open-data endpoint, which offers `unix` auth among others — and the
//! redirect, capability and lazy-read paths are exercised by the `#[ignore]`d
//! live tests below, which read a real 966 MB sample without fetching it whole.
#![cfg(feature = "xrootd")]

use oxiroot_io_core::cast::SaturatingFrom;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use oxiroot::prelude::*;
use oxiroot::tree::TreeReader;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

// Opcodes / statuses used by the mock.
const KXR_AUTH: u16 = 3000;
const KXR_CLOSE: u16 = 3003;
const KXR_LOGIN: u16 = 3007;
const KXR_OPEN: u16 = 3010;
const KXR_PROTOCOL: u16 = 3006;
const KXR_READ: u16 = 3013;
const KXR_READV: u16 = 3025;
const KXR_STAT: u16 = 3017;
const KXR_OK: u16 = 0;
const KXR_OKSOFAR: u16 = 4000;
const KXR_ERROR: u16 = 4003;
const KXR_WAIT: u16 = 4005;

/// How the mock server behaves: correctly, or like a hostile/broken server.
#[derive(Clone, Copy)]
enum Mode {
    Normal,
    /// Answer every open with `kXR_wait` (0 seconds), forever.
    WaitForever,
    /// Announce a 2 GB stat response, then send nothing.
    HugeFrame,
    /// Answer reads with fewer bytes than asked for.
    ShortRead,
    /// Announce a read's full length, send half of it, and hang up.
    DropMidResponse,
    /// Refuse every read with `kXR_error`.
    ReadError,
    /// Answer a vector read with each element's data at the wrong offset.
    ReadvWrongOffset,
    /// Answer a vector read with its last element cut short.
    ReadvTruncated,
    /// Answer a vector read with extra bytes after the last element.
    ReadvTrailing,
    /// Switch to TLS when the client can (`kXR_gotoTLS`), and offer token
    /// (`ztn`) then `unix` authentication.
    #[cfg_attr(not(feature = "xrootd-tls"), allow(dead_code))]
    Tls,
    /// Stay plain but offer only token (`ztn`) authentication.
    #[cfg_attr(not(feature = "xrootd-tls"), allow(dead_code))]
    ZtnOnly,
}

/// The `kXR_protocol` response flags for "TLS available, switch now".
const KXR_HAVE_TLS: u32 = 0x8000_0000;
const KXR_GOTO_TLS: u32 = 0x4000_0000;

/// What the mock was asked for.
#[derive(Default)]
struct Counts {
    reads: AtomicUsize,
    readvs: AtomicUsize,
    /// Each `kXR_auth`: its credential type and credential.
    auths: Mutex<Vec<([u8; 4], Vec<u8>)>>,
}

/// Serve `data` as a single `XRootD` data server on a background thread.
fn serve(data: Vec<u8>) -> String {
    serve_as(data, Mode::Normal)
}

fn serve_as(data: Vec<u8>, mode: Mode) -> String {
    serve_counting(data, mode).0
}

/// Serve `data`, returning the URL and the counts of what was asked for.
fn serve_counting(data: Vec<u8>, mode: Mode) -> (String, Arc<Counts>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let data = Arc::new(data);
    let counts = Arc::new(Counts::default());
    let counts_bg = Arc::clone(&counts);
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let data = Arc::clone(&data);
            let counts = Arc::clone(&counts_bg);
            std::thread::spawn(move || handle(stream, &data, mode, &counts));
        }
    });
    (format!("root://127.0.0.1:{port}//file.root"), counts)
}

/// The reply to a vector read: each element's 16-byte header (fhandle, rlen,
/// offset) then its bytes, bent the way `mode` says.
fn readv_reply(list: &[u8], data: &[u8], mode: Mode) -> Vec<u8> {
    let mut out = Vec::new();
    let n = list.len() / 16;
    for (k, el) in list.as_chunks::<16>().0.iter().enumerate() {
        let rlen = usize::saturating_from(i32::from_be_bytes(el[4..8].try_into().unwrap()).max(0));
        let offset =
            usize::saturating_from(i64::from_be_bytes(el[8..16].try_into().unwrap()).max(0));
        let end = (offset + rlen).min(data.len());
        let mut chunk = data.get(offset..end).unwrap_or(&[]).to_vec();
        let mut head = el.to_vec();
        if matches!(mode, Mode::ReadvWrongOffset) {
            head[8..16].copy_from_slice(&i64::saturating_from(offset + 1).to_be_bytes());
        }
        if matches!(mode, Mode::ReadvTruncated) && k + 1 == n {
            chunk.truncate(chunk.len() / 2);
        }
        out.extend_from_slice(&head);
        out.extend_from_slice(&chunk);
    }
    if matches!(mode, Mode::ReadvTrailing) {
        out.extend_from_slice(b"extra");
    }
    out
}

fn reply(stream: &mut impl Write, streamid: [u8; 2], status: u16, payload: &[u8]) -> bool {
    let mut msg = Vec::with_capacity(8 + payload.len());
    msg.extend_from_slice(&streamid);
    msg.extend_from_slice(&status.to_be_bytes());
    msg.extend_from_slice(&i32::saturating_from(payload.len()).to_be_bytes());
    msg.extend_from_slice(payload);
    stream.write_all(&msg).is_ok()
}

fn handle(mut stream: TcpStream, data: &[u8], mode: Mode, counts: &Counts) {
    // Handshake: read the 20-byte client handshake, reply 16 bytes (the client
    // ignores the content: two pad int32, protover, server flags = data server).
    let mut hs = [0u8; 20];
    if stream.read_exact(&mut hs).is_err() {
        return;
    }
    let mut reply16 = [0u8; 16];
    reply16[8..12].copy_from_slice(&0x0000_0511i32.to_be_bytes()); // protover
    reply16[12..16].copy_from_slice(&1i32.to_be_bytes()); // kXR_isServer (data server)
    if stream.write_all(&reply16).is_err() {
        return;
    }
    if let Next::StartTls = serve_requests(&mut stream, data, mode, counts, false) {
        let conn = rustls::ServerConnection::new(tls_server_config()).expect("TLS server");
        let mut tls = rustls::StreamOwned::new(conn, stream);
        serve_requests(&mut tls, data, mode, counts, true);
    }
}

/// The mock's TLS side: the test server certificate for `localhost` and
/// `127.0.0.1`, signed by the test CA in `fixtures/xrootd_tls`.
fn tls_server_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs = CertificateDer::pem_file_iter(fixture("xrootd_tls/server.pem"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_file(fixture("xrootd_tls/server.key")).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    Arc::new(config)
}

/// What the request loop ended with.
enum Next {
    /// The client hung up, or the mock stopped answering.
    Done,
    /// The mock answered `kXR_gotoTLS`: carry on over TLS.
    StartTls,
}

/// Answer requests on `stream` until the client goes away — or, unless
/// already `in_tls`, until a `kXR_protocol` exchange switches it to TLS.
fn serve_requests(
    stream: &mut (impl Read + Write),
    data: &[u8],
    mode: Mode,
    counts: &Counts,
    in_tls: bool,
) -> Next {
    loop {
        let mut head = [0u8; 24];
        if stream.read_exact(&mut head).is_err() {
            return Next::Done;
        }
        let streamid = [head[0], head[1]];
        let reqid = u16::from_be_bytes([head[2], head[3]]);
        let body = &head[4..20];
        let dlen = usize::saturating_from(
            i32::from_be_bytes([head[20], head[21], head[22], head[23]]).max(0),
        );
        let mut reqdata = vec![0u8; dlen];
        if stream.read_exact(&mut reqdata).is_err() {
            return Next::Done;
        }

        let ok = match reqid {
            KXR_PROTOCOL => {
                // pval + flags. The client says it can do TLS in the first
                // body byte after clientpv (kXR_ableTLS = 0x02).
                let able_tls = body[4] & 0x02 != 0;
                let switch = matches!(mode, Mode::Tls) && able_tls && !in_tls;
                let flags = if switch {
                    KXR_HAVE_TLS | KXR_GOTO_TLS
                } else {
                    0
                };
                let mut pl = 0x0000_0520i32.to_be_bytes().to_vec();
                pl.extend_from_slice(&flags.to_be_bytes());
                if !reply(stream, streamid, KXR_OK, &pl) {
                    return Next::Done;
                }
                if switch {
                    return Next::StartTls;
                }
                true
            }
            KXR_LOGIN => {
                // 16-byte session id + a security spec.
                let mut pl = vec![0u8; 16];
                pl.extend_from_slice(match mode {
                    Mode::Tls => b"&P=ztn,0:4096:&P=unix".as_slice(),
                    Mode::ZtnOnly => b"&P=ztn,0:4096:",
                    _ => b"&P=unix",
                });
                reply(stream, streamid, KXR_OK, &pl)
            }
            KXR_AUTH => {
                let credtype = body[12..16].try_into().unwrap();
                counts
                    .auths
                    .lock()
                    .unwrap()
                    .push((credtype, reqdata.clone()));
                reply(stream, streamid, KXR_OK, &[])
            }
            KXR_OPEN if matches!(mode, Mode::WaitForever) => {
                reply(stream, streamid, KXR_WAIT, &0i32.to_be_bytes())
            }
            KXR_OPEN => reply(stream, streamid, KXR_OK, &[0, 0, 0, 0]), // fhandle
            KXR_STAT if matches!(mode, Mode::HugeFrame) => {
                let mut head = Vec::new();
                head.extend_from_slice(&streamid);
                head.extend_from_slice(&KXR_OK.to_be_bytes());
                head.extend_from_slice(&2_000_000_000i32.to_be_bytes());
                let _ = stream.write_all(&head);
                return Next::Done; // and send nothing more
            }
            KXR_STAT => {
                // "id size flags modtime"
                let s = format!("0 {} 0 0", data.len());
                reply(stream, streamid, KXR_OK, s.as_bytes())
            }
            KXR_READ => {
                counts.reads.fetch_add(1, Ordering::Relaxed);
                // Body: fhandle[4] + offset(i64) + rlen(i32).
                let offset =
                    usize::saturating_from(i64::from_be_bytes(body[4..12].try_into().unwrap()));
                let rlen = usize::saturating_from(
                    i32::from_be_bytes(body[12..16].try_into().unwrap()).max(0),
                );
                let end = (offset + rlen).min(data.len());
                let chunk = data.get(offset..end).unwrap_or(&[]);
                // The file header and key list are read before any data, and
                // must succeed for the open to; misbehave on later reads.
                let misbehave = offset > 0 && rlen > 64;
                match mode {
                    Mode::ShortRead if misbehave => {
                        reply(stream, streamid, KXR_OK, &chunk[..chunk.len() / 2])
                    }
                    Mode::DropMidResponse if misbehave => {
                        let mut msg = Vec::new();
                        msg.extend_from_slice(&streamid);
                        msg.extend_from_slice(&KXR_OK.to_be_bytes());
                        msg.extend_from_slice(&i32::saturating_from(chunk.len()).to_be_bytes());
                        msg.extend_from_slice(&chunk[..chunk.len() / 2]);
                        let _ = stream.write_all(&msg);
                        return Next::Done; // hang up
                    }
                    Mode::ReadError if misbehave => {
                        let mut pl = 3011i32.to_be_bytes().to_vec(); // kXR_NotFound
                        pl.extend_from_slice(b"no such data\0");
                        reply(stream, streamid, KXR_ERROR, &pl)
                    }
                    _ => reply(stream, streamid, KXR_OK, chunk),
                }
            }
            KXR_READV => {
                counts.readvs.fetch_add(1, Ordering::Relaxed);
                // Two frames, the first a `kXR_oksofar` continuation, as a
                // server streaming a long reply sends it.
                let out = readv_reply(&reqdata, data, mode);
                let half = out.len() / 2;
                reply(stream, streamid, KXR_OKSOFAR, &out[..half])
                    && reply(stream, streamid, KXR_OK, &out[half..])
            }
            KXR_CLOSE => reply(stream, streamid, KXR_OK, &[]),
            _ => reply(stream, streamid, KXR_OK, &[]),
        };
        if !ok {
            return Next::Done;
        }
    }
}

/// A server that keeps answering `kXR_wait` makes the open fail after a
/// bounded number of retries (the client used to recurse once per reply).
#[test]
fn endless_kxr_wait_is_an_error() {
    let url = serve_as(Vec::new(), Mode::WaitForever);
    let err = FileReader::open_url(&url).unwrap_err();
    assert!(err.to_string().contains("wait"), "{err}");
}

/// A frame announcing far more than the request allows is a protocol error,
/// raised before anything is buffered.
#[test]
fn an_oversized_response_is_an_error_not_an_allocation() {
    let url = serve_as(Vec::new(), Mode::HugeFrame);
    let err = FileReader::open_url(&url).unwrap_err();
    assert!(err.to_string().contains("exceeds"), "{err}");
}

/// A tree whose two branches are split into 50 baskets each, written to a
/// temporary file and returned as bytes.
fn many_basket_tree() -> Vec<u8> {
    use crate::support::write_tree_file_baskets;
    use oxiroot::tree::raw::Branch;
    // A directory of its own per call: tests run in parallel and each removes
    // its directory when done.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "oxiroot-readv-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("baskets.root");
    let x: Vec<f64> = (0..5000).map(|i| f64::from(i) * 0.5).collect();
    let n: Vec<i32> = (0..5000).map(|i| i % 7).collect();
    write_tree_file_baskets(
        &path,
        "events",
        &[Branch::new("x", x), Branch::new("n", n)],
        Compression::Zstd(5),
        100,
    )
    .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

/// Reading a branch of many baskets fetches them with vector reads, a round
/// trip per batch, not a `kXR_read` each — and gets the same values.
#[test]
fn a_branch_of_many_baskets_is_read_in_a_few_round_trips() {
    let bytes = many_basket_tree();
    let local = FileReader::from_bytes(bytes.clone()).unwrap();
    let ltree = TreeReader::open(&local, "events").unwrap();
    let expected = ltree.branch("x").values().unwrap();

    let (url, counts) = serve_counting(bytes, Mode::Normal);
    let remote = FileReader::open_url(&url).unwrap();
    let tree = TreeReader::open(&remote, "events").unwrap();
    let reads_before = counts.reads.load(Ordering::Relaxed);
    let got = tree.branch("x").values().unwrap();
    assert_eq!(format!("{got:?}"), format!("{expected:?}"));
    let reads = counts.reads.load(Ordering::Relaxed) - reads_before;
    let readvs = counts.readvs.load(Ordering::Relaxed);
    assert_eq!(readvs, 1, "the 50 baskets in one vector read");
    assert_eq!(reads, 0, "no basket was read on its own");
}

/// A server that answers wrongly fails the read with an error, never a
/// panic, a hang, or wrong values.
#[test]
fn a_misbehaving_server_is_an_error() {
    let bytes = many_basket_tree();
    for (mode, why) in [
        (Mode::ReadvWrongOffset, "vector read returned"),
        (Mode::ReadvTruncated, "needed"),
        (Mode::ReadvTrailing, "exceeds"),
    ] {
        let url = serve_as(bytes.clone(), mode);
        let remote = FileReader::open_url(&url).unwrap();
        let tree = TreeReader::open(&remote, "events").unwrap();
        let err = tree.branch("x").values().unwrap_err();
        assert!(err.to_string().contains(why), "{why}: {err}");
    }
}

/// Plain reads that go wrong — too few bytes, a dropped connection, an error
/// status — are errors too. They are hit by a read outside any batch: a key
/// payload.
#[test]
fn a_failing_plain_read_is_an_error() {
    let bytes = std::fs::read(fixture("tree_flat.root")).expect("read fixture");
    for (mode, why) in [
        (Mode::ShortRead, ""),
        (Mode::DropMidResponse, ""),
        (Mode::ReadError, "no such data"),
    ] {
        let url = serve_as(bytes.clone(), mode);
        // The open itself reads past the header (streamer info, keys): any of
        // these modes must surface as an error, wherever it first bites.
        let result = FileReader::open_url(&url).and_then(|f| {
            let name = f
                .keys()
                .iter()
                .find(|k| k.class_name == "TTree")
                .map(|k| k.name.clone())
                .expect("a TTree key");
            let tree = TreeReader::open(&f, &name)?;
            let branch = tree.branch_names().collect::<Vec<_>>()[0].to_string();
            tree.branch(&branch).values().map(|_| ())
        });
        let err = result.expect_err("a broken server cannot yield data");
        assert!(err.to_string().contains(why), "{err}");
    }
}

#[test]
fn reads_a_ttree_over_xrootd() {
    let bytes = std::fs::read(fixture("tree_flat.root")).expect("read fixture");

    // Local reference.
    let local = FileReader::from_bytes(bytes.clone()).expect("local open");
    let tkey = local
        .keys()
        .iter()
        .find(|k| k.class_name == "TTree")
        .expect("a TTree key");
    let tname = tkey.name.clone();
    let ltree = TreeReader::open(&local, &tname).expect("local tree");
    let branch = ltree.branch_names().collect::<Vec<_>>()[0].to_string();
    let local_vals = format!(
        "{:?}",
        ltree.branch(&branch).values().expect("local branch")
    );

    // Same file over the XRootD protocol.
    let url = serve(bytes.clone());
    let remote = FileReader::open_url(&url).expect("open_url root://");
    assert_eq!(remote.size(), bytes.len() as u64, "size via fstat");
    let rkeys: Vec<String> = remote.keys().iter().map(|k| k.name.clone()).collect();
    let lkeys: Vec<String> = local.keys().iter().map(|k| k.name.clone()).collect();
    assert_eq!(rkeys, lkeys, "same keys over root://");

    let rtree = TreeReader::open(&remote, &tname).expect("remote tree");
    assert_eq!(rtree.entries(), ltree.entries());
    let remote_vals = format!(
        "{:?}",
        rtree.branch(&branch).values().expect("remote branch")
    );
    assert_eq!(remote_vals, local_vals, "branch values match over root://");
}

/// Live read of a real public file from CERN's EOS over `root://` (unix auth,
/// redirect + capability, lazy reads). Ignored by default: it needs network and
/// the public data servers are slow/intermittent from outside CERN. Run with
/// `cargo test -p oxiroot --features xrootd -- --ignored xrootd_live`.
#[test]
#[ignore]
fn xrootd_live_eospublic() {
    let url = "root://eospublic.cern.ch//eos/root-eos/hsimple.root";
    let f = FileReader::open_url(url).expect("open eospublic");
    assert!(f.size() > 100_000, "hsimple.root is ~400 KiB");
    let names: Vec<String> = f.keys().iter().map(|k| k.name.clone()).collect();
    assert!(names.contains(&"hpx".to_string()), "keys: {names:?}");
}

/// The same endpoint, but reading *data* out of a file far too large to fetch
/// whole: an ATLAS open-data sample of 966 MB, where opening the file, reading
/// the tree's metadata and reading one branch together touch only a few ranges.
/// This is the check that the lazy path holds against a real server — a client
/// that quietly downloaded the file would take minutes here rather than seconds.
///
/// `eospublic.cern.ch` is a manager node, so the read also goes through the
/// redirect and the capability it hands back. Ignored by default, like the smoke
/// test above; run it the same way.
#[test]
#[ignore]
fn xrootd_live_eospublic_reads_a_tree() {
    let url = "root://eospublic.cern.ch//eos/opendata/atlas/OutreachDatasets/\
               2016-07-29/MC/mc_147770.Zee.root";
    let url: String = url.chars().filter(|c| !c.is_whitespace()).collect();

    let file = FileReader::open_url(&url).expect("open the ATLAS sample");
    assert!(file.size() > 900_000_000, "the sample is about 966 MB");

    let tree = TreeReader::open(&file, "mini").expect("the mini tree");
    assert_eq!(tree.entries(), 7_500_000);
    assert_eq!(tree.branch_names().len(), 46);

    // One branch, one entry window: the leptons per event of a Z→ee sample,
    // which are one or two.
    let leptons = tree
        .branch("lep_n")
        .range(0..20)
        .values()
        .expect("read lep_n");
    let counts = leptons.as_u32().expect("lep_n is a uint32 branch");
    assert_eq!(counts.len(), 20);
    assert!(
        counts.iter().all(|&n| (1..=2).contains(&n)),
        "a Z→ee event has one or two leptons: {counts:?}"
    );
}

/// An `RNTuple` over several clusters reads the same over `root://`, its pages
/// fetched by vector reads.
#[test]
fn reads_a_multi_cluster_rntuple_over_xrootd() {
    use oxiroot::ntuple::NtupleReader;
    let bytes = std::fs::read(fixture("rntuple_multicluster_vec.root")).unwrap();
    let local = FileReader::from_bytes(bytes.clone()).unwrap();
    let lnt = NtupleReader::open(&local, "ntpl").unwrap();

    let (url, counts) = serve_counting(bytes, Mode::Normal);
    let remote = FileReader::open_url(&url).unwrap();
    let rnt = NtupleReader::open(&remote, "ntpl").unwrap();
    for field in ["n", "v"] {
        assert_eq!(
            rnt.field(field).values().unwrap(),
            lnt.field(field).values().unwrap(),
            "{field}"
        );
    }
    assert!(
        counts.readvs.load(Ordering::Relaxed) > 0,
        "pages came by vector read"
    );
}

/// Many baskets from a real server: 300 000 entries of one branch of the
/// ATLAS sample, fetched by vector reads. Checks the `kXR_readv` framing
/// against EOS (the hermetic tests only check it against the mock). Ignored
/// by default; run as the live tests above.
#[test]
#[ignore]
fn xrootd_live_eospublic_reads_many_baskets() {
    let url: String = "root://eospublic.cern.ch//eos/opendata/atlas/OutreachDatasets/\
                       2016-07-29/MC/mc_147770.Zee.root"
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let file = FileReader::open_url(&url).expect("open the ATLAS sample");
    let tree = TreeReader::open(&file, "mini").expect("the mini tree");
    let start = std::time::Instant::now();
    let leptons = tree
        .branch("lep_n")
        .range(0..300_000)
        .values()
        .expect("read lep_n");
    let counts = leptons.as_u32().expect("lep_n is a uint32 branch");
    assert_eq!(counts.len(), 300_000);
    assert!(counts.iter().all(|&n| n <= 10), "plausible lepton counts");
    eprintln!("300 000 entries of lep_n in {:?}", start.elapsed());
}

/// `roots://` is refused without the `xrootd-tls` feature rather than read
/// over a plain connection.
#[cfg(not(feature = "xrootd-tls"))]
#[test]
fn roots_needs_the_tls_feature() {
    let err = FileReader::open_url("roots://127.0.0.1:1//file.root").unwrap_err();
    assert!(err.to_string().contains("xrootd-tls"), "{err}");
}

/// TLS and token authentication against the mock (`xrootd-tls` feature): the
/// server's certificate is checked against the test CA, and the token goes
/// out as a `ztn` credential.
#[cfg(feature = "xrootd-tls")]
mod tls {
    use super::*;
    use oxiroot::file::{XrootdOptions, XrootdSource};

    const TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ0ZXN0In0.c2ln";

    fn opts() -> XrootdOptions {
        XrootdOptions::new()
            .token(format!("  {TOKEN}\n"))
            .ca_dir(fixture("xrootd_tls"))
    }

    fn open(url: &str, opts: &XrootdOptions) -> oxiroot::Result<FileReader> {
        let source = XrootdSource::open_with(url, opts)?;
        Ok(FileReader::from_source(Box::new(source))?)
    }

    /// The `ztn` credential: header, big-endian length with the null byte,
    /// the token, a null byte.
    fn ztn_credential(token: &str) -> Vec<u8> {
        let mut c = b"ztn\0\0T\0\0".to_vec();
        c.extend_from_slice(&u16::try_from(token.len() + 1).unwrap().to_be_bytes());
        c.extend_from_slice(token.as_bytes());
        c.push(0);
        c
    }

    fn check_token_auth(url: &str, counts: &Counts) {
        let data = std::fs::read(fixture("th1d_zstd.root")).unwrap();
        let file = open(url, &opts()).unwrap();
        let local = FileReader::from_bytes(data).unwrap();
        assert_eq!(file.keys().len(), local.keys().len());
        let auths = counts.auths.lock().unwrap();
        assert_eq!(auths.len(), 1, "one kXR_auth");
        assert_eq!(&auths[0].0, b"ztn\0");
        assert_eq!(auths[0].1, ztn_credential(TOKEN), "trimmed token");
    }

    /// A `roots://` URL goes to TLS before the login and authenticates with
    /// the token, the server's first choice.
    #[test]
    fn roots_authenticates_with_a_token() {
        let data = std::fs::read(fixture("th1d_zstd.root")).unwrap();
        let (url, counts) = serve_counting(data, Mode::Tls);
        check_token_auth(&url.replace("root://", "roots://"), &counts);
    }

    /// A `root://` URL switches to TLS when the server asks for it.
    #[test]
    fn root_switches_to_tls_when_the_server_asks() {
        let data = std::fs::read(fixture("th1d_zstd.root")).unwrap();
        let (url, counts) = serve_counting(data, Mode::Tls);
        check_token_auth(&url, &counts);
    }

    /// A plain server keeps working: no TLS, `unix` auth.
    #[test]
    fn a_plain_server_still_uses_unix_auth() {
        let data = std::fs::read(fixture("th1d_zstd.root")).unwrap();
        let (url, counts) = serve_counting(data, Mode::Normal);
        open(&url, &opts()).unwrap();
        let auths = counts.auths.lock().unwrap();
        assert_eq!(auths.len(), 1);
        assert_eq!(&auths[0].0, b"unix");
    }

    /// `roots://` refuses a server that cannot do TLS.
    #[test]
    fn roots_needs_a_server_with_tls() {
        let url = serve(Vec::new()).replace("root://", "roots://");
        let err = open(&url, &opts()).unwrap_err();
        assert!(err.to_string().contains("does not offer TLS"), "{err}");
    }

    /// The token never goes out over a plain connection.
    #[test]
    fn a_token_needs_tls() {
        let (url, counts) = serve_counting(Vec::new(), Mode::ZtnOnly);
        let err = open(&url, &opts()).unwrap_err();
        assert!(err.to_string().contains("needs TLS"), "{err}");
        assert!(counts.auths.lock().unwrap().is_empty(), "no kXR_auth sent");
    }

    /// A server certificate from a CA the client does not trust ends the
    /// connection in the handshake, before any credential is sent.
    #[test]
    fn an_untrusted_certificate_is_an_error() {
        let (url, counts) = serve_counting(Vec::new(), Mode::Tls);
        let empty = std::env::temp_dir().join(format!("oxiroot-no-ca-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let opts = XrootdOptions::new().token(TOKEN).ca_dir(&empty);
        let err = open(&url.replace("root://", "roots://"), &opts).unwrap_err();
        let _ = std::fs::remove_dir_all(&empty);
        assert!(err.to_string().contains("TLS handshake"), "{err}");
        assert!(counts.auths.lock().unwrap().is_empty(), "no kXR_auth sent");
    }

    /// A real `xrootd` with TLS and token authentication, as
    /// `scripts/xrootd_tls_local.sh` sets one up: `OXIROOT_ROOTS_URL` names a
    /// file on it, and the token comes from the environment
    /// (`BEARER_TOKEN`, …) the WLCG way.
    #[test]
    #[ignore = "needs a local xrootd with TLS: scripts/xrootd_tls_local.sh"]
    fn live_xrootd_with_tls_and_a_token() {
        let url = std::env::var("OXIROOT_ROOTS_URL").expect("OXIROOT_ROOTS_URL");
        let opts = XrootdOptions::new().ca_dir(fixture("xrootd_tls"));
        let file = open(&url, &opts).unwrap();
        assert!(!file.keys().is_empty());
    }
}
