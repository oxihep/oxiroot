//! Interop driver for the round-trip CI: writes a canonical histogram + `RNTuple`
//! for an external ROOT reader (uproot / ROOT C++) to verify, and reads the
//! files an external ROOT writer produced, asserting the same canonical values.
//!
//!   cargo run -p oxiroot --example interop -- write <dir>   # Rust → ROOT
//!   cargo run -p oxiroot --example interop -- read  <dir>   # ROOT → Rust
//!
//! Canonical dataset (both sides agree):
//!   - TH1D "h": 4 bins over [0, 4), in-range bin contents [1, 2, 3, 4].
//!   - `RNTuple` "ntpl": field x = int32 [1,2,3,4,5], y = double [1.5,2.5,3.5,4.5,5.5].
//!   - `TTree` "Tree": ti = int32 [1..5], tf = double [1.5..5.5], tv = double[3]
//!     fixed array, ts = string, tj = jagged double (auto count ntj),
//!     tw = `std::vector`<double> (`TBranchElement`), th = split `std::vector`<Hit>
//!     (Hit = {float x; float y; int id;}) read back as th.x/th.y/th.id.
//!   - `rust_multi.root` (`FileWriter`, Rust → oracle): top-level TH1D "mh"
//!     = [5,6,7], a subdirectory "sub" holding TH1D "sh" = [8,9], and a
//!     directory nested three deep, "sub/deep/deeper" holding TH1D "dh2"
//!     = [10,11].
//!   - `rust_append.root` (`FileWriter` create then open/append, Rust → oracle):
//!     TH1D "bh" = [3,1] written first, TH1D "ah" = [4] appended afterwards.
//!   - `rust_codec`_<zlib|lz4|zstd|lzma>.root (Rust → oracle), one file per
//!     codec, each holding: the canonical TH1D "h"; a TH2D "h2" of 3×2 unit
//!     bins over [0,3)×[0,2) with in-range bin (i, j) = 1 + i + 3j; a TH3D
//!     "h3" of 2×2×2 unit bins with bin (i, j, k) = 1 + i + 2j + 4k; a
//!     `TProfile` "p" of 2 unit bins with means [2, 5] from entries [2, 1]; and
//!     a TH1D "big" of 1000 unit bins with bin i = i % 7, large enough that
//!     every codec really compresses it.
//!   - `oracle_dirs.root` (oracle → Rust, read via `FileReader::get`):
//!     top-level TH1D "dh" = [2,4] plus a subdirectory "region" holding "rh"
//!     = [3,6,9].

use oxiroot_io_core::cast::SaturatingFrom;
use std::path::Path;
use std::process::exit;

use oxiroot::ntuple::NtupleReader;
use oxiroot::ntuple::raw::{Field, FieldValues};
use oxiroot::prelude::*;
use oxiroot::tree::TreeReader;
use oxiroot::tree::raw::{Branch, Column, SplitMember};

#[path = "support/whole_file.rs"]
mod whole_file;
use whole_file::{write_rntuple_file, write_tree_file};

/// In-range bin contents of the canonical histogram.
const HIST_BINS: [f64; 4] = [1.0, 2.0, 3.0, 4.0];
/// Canonical `RNTuple` columns.
const NTPL_X: [i32; 5] = [1, 2, 3, 4, 5];
const NTPL_Y: [f64; 5] = [1.5, 2.5, 3.5, 4.5, 5.5];
/// Canonical `TTree` fixed-array (`tv`) and string (`ts`) columns.
const TREE_TV: [[f64; 3]; 5] = [
    [1.0, 2.0, 3.0],
    [4.0, 5.0, 6.0],
    [7.0, 8.0, 9.0],
    [10.0, 11.0, 12.0],
    [13.0, 14.0, 15.0],
];
const TREE_TS: [&str; 5] = ["a", "bb", "ccc", "dddd", "eeeee"];
/// Canonical `TTree` jagged (variable-length) column.
const TREE_TJ: [&[f64]; 5] = [&[1.0], &[2.0, 3.0], &[], &[4.0, 5.0, 6.0], &[7.0]];
/// Canonical `TTree` `std::vector<double>` (`TBranchElement`) column.
const TREE_TW: [&[f64]; 5] = [
    &[10.0, 20.0],
    &[],
    &[30.0],
    &[40.0, 50.0],
    &[60.0, 70.0, 80.0],
];
/// Canonical split `std::vector<Hit>` branch `th` (Hit = {float x; float y; int
/// id;}), exposed by ROOT as the per-member sub-branches `th.x`/`th.y`/`th.id`.
/// Per-entry element counts [1, 0, 2, 1, 3] (includes an empty entry).
const TREE_TH_X: [&[f32]; 5] = [&[1.0], &[], &[2.0, 3.0], &[4.0], &[5.0, 6.0, 7.0]];
const TREE_TH_Y: [&[f32]; 5] = [&[1.5], &[], &[2.5, 3.5], &[4.5], &[5.5, 6.5, 7.5]];
const TREE_TH_ID: [&[i32]; 5] = [&[1], &[], &[2, 3], &[4], &[5, 6, 7]];
/// Canonical oracle-written `TTree` "otree" (ROOT/uproot → Rust): a scalar (`oi`),
/// a jagged double (`oj`), a string (`os`), and a `std::vector<double>` (`ov`).
/// uproot cannot write `std::vector`, so `ov` is present only in the ROOT C++
/// oracle and is read back only when the branch exists.
const OTREE_OI: [i32; 3] = [10, 11, 12];
const OTREE_OJ: [&[f64]; 3] = [&[1.0, 2.0], &[], &[3.0]];
const OTREE_OS: [&str; 3] = ["x", "yy", "zzz"];
const OTREE_OV: [&[f64]; 3] = [&[1.0], &[2.0, 3.0], &[]];
/// `rust_multi.root` (FileWriter): top-level `mh`, subdirectory `sub/sh`, and
/// `sub/deep/deeper/dh2` three levels down.
const MULTI_MH: [f64; 3] = [5.0, 6.0, 7.0];
const MULTI_SH: [f64; 2] = [8.0, 9.0];
const MULTI_DH2: [f64; 2] = [10.0, 11.0];
/// `rust_append.root`: base `bh`, then `ah` appended via `FileUpdater::open`.
const APPEND_BH: [f64; 2] = [3.0, 1.0];
const APPEND_AH: [f64; 1] = [4.0];
/// Oracle-written `oracle_dirs.root`: top-level `dh` + subdirectory `region/rh`.
const DIRS_DH: [f64; 2] = [2.0, 4.0];
const DIRS_RH: [f64; 3] = [3.0, 6.0, 9.0];

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a small whole fill count"
)]
fn canonical_hist() -> Hist1D {
    // Filled as an oxihist histogram and converted afterwards, so ROOT and
    // uproot reading it checks the oxihist → ROOT conversion end to end.
    let mut h = Hist::reg(4, 0.0, 4.0).name("h").title("interop").double();
    // Fill bin i (0-based) with i+1 entries at its center, giving contents
    // [1, 2, 3, 4].
    for (i, &count) in HIST_BINS.iter().enumerate() {
        let center = i as f64 + 0.5;
        for _ in 0..(count as usize) {
            h.fill(center);
        }
    }
    h.to_root().unwrap()
}

/// A `TH1D` with `contents.len()` unit bins over `[0, n)` whose in-range bin `i`
/// holds `contents[i]` (one weighted fill per bin). Used for the directory /
/// multi-object / append fixtures.
fn hist(name: &str, contents: &[f64]) -> Hist1D {
    let n = i32::saturating_from(contents.len());
    let mut h = Hist::reg(usize::saturating_from(n), 0.0, f64::from(n))
        .weight()
        .named(name)
        .titled("interop");
    for (i, &c) in contents.iter().enumerate() {
        h.fill_weight(i as f64 + 0.5, c);
    }
    h.to_root().expect("a regular axis maps to ROOT")
}

/// The codecs of the matrix, by file-name suffix.
const CODECS: [(&str, Compression); 4] = [
    ("zlib", Compression::Zlib(6)),
    ("lz4", Compression::Lz4(4)),
    ("zstd", Compression::Zstd(5)),
    ("lzma", Compression::Lzma(6)),
];

/// The 2-D, 3-D, profile and large histograms of the codec matrix.
fn matrix_hists() -> (Hist2D, Hist3D, Profile1D, Hist1D) {
    let mut h2 = Hist::reg(3, 0.0, 3.0).reg(2, 0.0, 2.0).weight().named("h2");
    for i in 0..3 {
        for j in 0..2 {
            h2.fill_weight(
                f64::from(i) + 0.5,
                f64::from(j) + 0.5,
                f64::from(1 + i + 3 * j),
            );
        }
    }
    let mut h3 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .weight()
        .named("h3");
    for i in 0..2 {
        for j in 0..2 {
            for k in 0..2 {
                let w = f64::from(1 + i + 2 * j + 4 * k);
                h3.fill_weight(
                    f64::from(i) + 0.5,
                    f64::from(j) + 0.5,
                    f64::from(k) + 0.5,
                    w,
                );
            }
        }
    }
    let mut p = Hist::reg(2, 0.0, 2.0).mean().named("p");
    p.fill_sample(0.5, 1.0);
    p.fill_sample(0.5, 3.0);
    p.fill_sample(1.5, 5.0);
    let mut big = Hist::reg(1000, 0.0, 1000.0).weight().named("big");
    for i in 0..1000 {
        big.fill_weight(f64::from(i) + 0.5, f64::from(i % 7));
    }
    (
        h2.to_root().expect("maps to ROOT"),
        h3.to_root().expect("maps to ROOT"),
        p.to_root().expect("maps to ROOT"),
        big.to_root().expect("maps to ROOT"),
    )
}

fn write(dir: &Path) -> oxiroot::Result<()> {
    let h = canonical_hist();
    FileWriter::create(dir.join("rust_hist.root"))
        .add(&h)
        .compression(Compression::Stored)
        .write()?;

    let fields = vec![
        Field::i32("x", NTPL_X.to_vec()),
        Field::f64("y", NTPL_Y.to_vec()),
    ];
    write_rntuple_file(
        dir.join("rust_ntuple.root"),
        "ntpl",
        &fields,
        Compression::Stored,
    )?;

    write_tree_file(
        dir.join("rust_tree.root"),
        "Tree",
        &[
            Branch::new("ti", NTPL_X.to_vec()),
            Branch::new("tf", NTPL_Y.to_vec()),
            Branch::new("tv", TREE_TV.iter().map(|r| r.to_vec()).collect() /*ARRAY_FIXME f64*/),
            Branch::new(
                "ts",
                TREE_TS
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect()),
            Branch::new("tj", TREE_TJ.iter().map(|r| r.to_vec()).collect()).counted(),
            Branch::new("tw", TREE_TW.iter().map(|r| r.to_vec()).collect()),
            Branch::split_vector(
                "th",
                "Hit",
                vec![
                    SplitMember::f32("x", TREE_TH_X.iter().map(|r| r.to_vec()).collect()),
                    SplitMember::f32("y", TREE_TH_Y.iter().map(|r| r.to_vec()).collect()),
                    SplitMember::i32("id", TREE_TH_ID.iter().map(|r| r.to_vec()).collect()),
                ],
            ),
        ],
        Compression::Stored,
    )?;

    // rust_multi.root — FileWriter: a top-level hist, a subdirectory (`sub`)
    // holding its own hist, and directories nested three deep inside it.
    let mh = hist("mh", &MULTI_MH);
    let sh = hist("sh", &MULTI_SH);
    let dh2 = hist("dh2", &MULTI_DH2);
    FileWriter::create(dir.join("rust_multi.root"))
        .add(&mh)
        .dir("sub", |d| {
            d.add(&sh).dir("deep", |d| d.dir("deeper", |d| d.add(&dh2)))
        })
        .compression(Compression::Stored)
        .write()?;

    // rust_append.root — write one hist, then append a second via FileUpdater::open
    // (the existing key is preserved and the new one added).
    let append_path = dir.join("rust_append.root");
    FileWriter::create(&append_path)
        .add(&hist("bh", &APPEND_BH))
        .compression(Compression::Stored)
        .write()?;
    oxiroot::FileUpdater::open(&append_path)?
        .add(&hist("ah", &APPEND_AH))
        .compression(Compression::Stored)
        .write()?;

    // rust_codec_*.root — the same objects under every codec.
    let (h2, h3, p, big) = matrix_hists();
    for (name, codec) in CODECS {
        FileWriter::create(dir.join(format!("rust_codec_{name}.root")))
            .add(&h)
            .add(&h2)
            .add(&h3)
            .add(&p)
            .add(&big)
            .compression(codec)
            .write()?;
    }

    println!(
        "wrote rust_hist.root + rust_ntuple.root + rust_tree.root + rust_multi.root \
         + rust_append.root + rust_codec_{{zlib,lz4,zstd,lzma}}.root to {}",
        dir.display()
    );
    Ok(())
}

fn read(dir: &Path) -> oxiroot::Result<()> {
    // Histogram written by the ROOT oracle.
    let f = FileReader::open(dir.join("oracle_hist.root"))?;
    let h = f.get::<Hist1D>("h")?;
    assert_close("hist bin contents", h.values(), &HIST_BINS);
    // The same histogram read straight into oxihist (ROOT → oxihist).
    let oh = f.get::<Hist1D>("h")?.to_hist()?;
    assert_close("hist via oxihist", &oh.values(false), &HIST_BINS);
    println!("read oracle_hist.root — bin contents match");

    // TTree written by the oracle (both ROOT C++ and uproot produce it).
    read_oracle_tree(dir)?;

    // Directory file written by the oracle: a top-level hist (`dh`, via
    // read_root) and a hist inside subdirectory `region` (`rh`, via
    // read_root_in). Both ROOT C++ and uproot produce it.
    let f = FileReader::open(dir.join("oracle_dirs.root"))?;
    assert_close("dirs dh", f.get::<Hist1D>("dh")?.values(), &DIRS_DH);
    assert_close(
        "dirs region/rh",
        f.get::<Hist1D>("region/rh")?.values(),
        &DIRS_RH,
    );
    println!("read oracle_dirs.root — top-level + subdirectory match");

    // RNTuple written by the ROOT oracle. uproot's RNTuple writer is
    // experimental, so the uproot job omits this file; only the ROOT C++ job
    // produces it. Skip the check when it is absent.
    let ntuple_path = dir.join("oracle_ntuple.root");
    if !ntuple_path.exists() {
        println!("oracle_ntuple.root absent — skipping RNTuple read check");
        return Ok(());
    }
    let f = FileReader::open(ntuple_path)?;
    let ntpl = NtupleReader::open(&f, "ntpl")?;
    match ntpl.field("x").values()? {
        FieldValues::I32(v) => assert_eq_or_die("ntuple x", &v, &NTPL_X),
        other => die(&format!("ntuple x: expected I32, got {other:?}")),
    }
    match ntpl.field("y").values()? {
        FieldValues::F64(v) => assert_close("ntuple y", &v, &NTPL_Y),
        other => die(&format!("ntuple y: expected F64, got {other:?}")),
    }
    println!("read oracle_ntuple.root — values match");
    Ok(())
}

fn read_oracle_tree(dir: &Path) -> oxiroot::Result<()> {
    let f = FileReader::open(dir.join("oracle_tree.root"))?;
    let t = TreeReader::open(&f, "otree")?;
    match t.branch("oi").values()? {
        Column::I32(v) => assert_eq_or_die("otree oi", &v, &OTREE_OI),
        other => die(&format!("otree oi: expected I32, got {other:?}")),
    }
    match t.branch("oj").values()? {
        Column::from(v) => assert_nested("otree oj", &v, &OTREE_OJ),
        other => die(&format!("otree oj: expected VecF64, got {other:?}")),
    }
    match t.branch("os").values()? {
        Column::from(v) => {
            let got: Vec<&str> = v.iter().map(String::as_str).collect();
            assert_eq_or_die("otree os", &got, &OTREE_OS);
        }
        other => die(&format!("otree os: expected Str, got {other:?}")),
    }
    // The std::vector<double> branch is written only by the ROOT C++ oracle.
    if t.branch_names().collect::<Vec<_>>().contains(&"ov") {
        match t.branch("ov").values()? {
            Column::from(v) => assert_nested("otree ov", &v, &OTREE_OV),
            other => die(&format!("otree ov: expected VecF64, got {other:?}")),
        }
    }
    println!("read oracle_tree.root — values match");
    Ok(())
}

fn assert_close(what: &str, got: &[f64], want: &[f64]) {
    if got.len() != want.len() || got.iter().zip(want).any(|(a, b)| (a - b).abs() > 1e-9) {
        die(&format!("{what}: got {got:?}, want {want:?}"));
    }
}

/// Assert a nested `Vec<Vec<f64>>` matches the canonical jagged/vector rows.
fn assert_nested(what: &str, got: &[Vec<f64>], want: &[&[f64]]) {
    let ok = got.len() == want.len()
        && got.iter().zip(want).all(|(g, w)| {
            g.len() == w.len() && g.iter().zip(w.iter()).all(|(a, b)| (a - b).abs() < 1e-9)
        });
    if !ok {
        die(&format!("{what}: got {got:?}, want {want:?}"));
    }
}

fn assert_eq_or_die<T: PartialEq + std::fmt::Debug>(what: &str, got: &[T], want: &[T]) {
    if got != want {
        die(&format!("{what}: got {got:?}, want {want:?}"));
    }
}

fn die(msg: &str) -> ! {
    eprintln!("interop MISMATCH: {msg}");
    exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = || -> ! {
        eprintln!(
            "usage: interop <write|read> <dir>\n\n\
             One half of a round trip with ROOT C++ or uproot. To run the whole\n\
             check locally (both directions, both oracles): bash scripts/interop_local.sh"
        );
        exit(2);
    };
    if args.len() != 3 {
        usage();
    }
    let dir = Path::new(&args[2]);
    let result = match args[1].as_str() {
        "write" => write(dir),
        "read" => read(dir),
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("interop ERROR: {e}");
        exit(1);
    }
}
