//! #3: a `Profile1D` survives write→read against real ROOT-produced data, and a
//! create→fill→save→read cycle round-trips through our own reader.

use oxiroot_hist::ToRoot;
use std::path::PathBuf;

use oxiroot_hist::{Hist, Profile1D};
use oxiroot_io_core::FileReader;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

#[test]
fn round_trips_real_root_tprofile() {
    let f = FileReader::open(fixture("tprofile_uncompressed.root")).expect("open fixture");
    let h = f.get::<Profile1D>("p").expect("read TProfile");

    let out = PathBuf::from("/tmp/rootrs_roundtrip_tprofile.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h)
        .compression(oxiroot_io_core::Compression::Stored)
        .write()
        .expect("write");
    let f2 = FileReader::open(&out).expect("reopen");
    let h2 = f2.get::<Profile1D>("p").expect("read back");
    assert_eq!(h2, h, "real ROOT TProfile must survive write→read");
}

#[test]
fn create_fill_save_round_trips() {
    // Reproduce the fixture's fills: 4 bins over [0,4], profiling y vs x.
    let mut h = Hist::reg(4, 0.0, 4.0).mean().named("p").titled("profile");
    h.fill_sample(0.5, 20.0);
    h.fill_sample(0.5, 10.0);
    h.fill_sample(1.5, 10.0);
    h.fill_sample(1.5, 5.0);
    h.fill_sample(2.5, 30.0);
    let h = h.to_root().unwrap();

    assert_eq!(h.entries(), 5.0);
    assert_eq!(h.tsumw(), 5.0);
    assert_eq!(h.tsumwy(), 75.0, "sum of y");
    assert_eq!(h.tsumwy2(), 1525.0, "sum of y^2");
    // values() = mean y per bin: bin1 = (20+10)/2 = 15, bin2 = (10+5)/2 = 7.5, bin3 = 30.
    assert_eq!(h.values(), [15.0, 7.5, 30.0, 0.0]);
    assert_eq!(h.bin_entries()[1..4], [2.0, 2.0, 1.0]);

    let out = PathBuf::from("/tmp/rootrs_filled_tprofile.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h)
        .compression(oxiroot_io_core::Compression::Stored)
        .write()
        .expect("write");
    let f = FileReader::open(&out).expect("reopen");
    let h2 = f.get::<Profile1D>("p").expect("read back");
    assert_eq!(h2, h, "filled profile must round-trip");
}

#[test]
fn writes_a_zstd_compressed_tprofile() {
    let f = FileReader::open(fixture("tprofile_uncompressed.root")).expect("open fixture");
    let h = f.get::<Profile1D>("p").expect("read TProfile");

    let out = PathBuf::from("/tmp/rootrs_written_tprofile_zstd.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h)
        .write()
        .expect("write compressed file");

    let f2 = FileReader::open(&out).expect("reopen");
    let key = f2.key("p").expect("p key");
    assert!(!key.is_uncompressed(), "object should be stored compressed");
    let h2 = f2
        .get::<Profile1D>("p")
        .expect("read back compressed TProfile");
    assert_eq!(h2, h, "compressed profile must round-trip");
}
