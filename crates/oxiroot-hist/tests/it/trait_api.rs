//! The idiomatic trait API (`WriteRoot`/`ReadRoot`/`BinContentType`) must select the
//! correct ROOT class for each bin content type (so ROOT compatibility is preserved)
//! and round-trip through a real file.

use oxiroot_hist::ToRoot;
use oxiroot_hist::{BinContentType, Hist, Hist1D, Hist2D, Profile1D};
use oxiroot_io_core::FileReader;
use oxiroot_io_core::{Compression, FileWriter, WriteRoot};

fn sample() -> Hist1D {
    let mut h = Hist::reg(10, 0.0, 10.0).weight().named("h").titled("title");
    for i in 0..10 {
        h.fill_weight(f64::from(i) + 0.5, f64::from(i + 1));
    }
    h.to_root().unwrap()
}

#[test]
fn trait_selects_correct_class_per_bin_content_type() {
    // `WriteRoot` is the sole byte producer; verify it tags each object with the
    // ROOT class its bin content type implies (round-trip byte correctness is covered by
    // the `*_round_trips*` tests below and the uproot/ROOT-C++ interop matrix).
    let h = sample();
    assert_eq!(h.root_class(), "TH1D");
    assert_eq!(h.bin_content_type(), BinContentType::F64);

    let hf = sample().with_bin_content_type(BinContentType::F32);
    assert_eq!(hf.root_class(), "TH1F");
    assert_eq!(hf.bin_content_type(), BinContentType::F32);
}

#[test]
fn write_root_then_read_root_round_trips() {
    let h = sample();
    let path = std::env::temp_dir().join("oxiroot_traitapi_h.root");
    FileWriter::create(&path)
        .add(&h)
        .compression(Compression::Zstd(3))
        .write()
        .expect("write_root");

    let f = FileReader::open(&path).expect("open");
    let back = f.get::<Hist1D>("h").expect("read_root");
    assert_eq!(back.values(), h.values());
    assert_eq!(back.name, "h");
    assert_eq!(back.class_name(), "TH1D");
}

#[test]
fn float_precision_round_trips_as_th1f() {
    let h = sample().with_bin_content_type(BinContentType::F32);
    let path = std::env::temp_dir().join("oxiroot_traitapi_hf.root");
    FileWriter::create(&path)
        .add(&h)
        .compression(Compression::Stored)
        .write()
        .expect("write");
    let f = FileReader::open(&path).expect("open");
    let back = f.get::<Hist1D>("h").expect("read");
    assert_eq!(back.class_name(), "TH1F"); // bin content type preserved on round-trip
}

#[test]
fn write_root_file_handles_heterogeneous_objects() {
    // The new multi-object writer takes any mix of writable types via &dyn —
    // not just Hist1D/Hist2D/Hist3D as the old Hist enum did.
    let h1 = Hist::reg(5, 0.0, 5.0)
        .double()
        .to_root()
        .unwrap()
        .named("h1");
    let h2 = Hist::reg(4, 0.0, 4.0)
        .reg(4, 0.0, 4.0)
        .double()
        .to_root()
        .unwrap()
        .named("h2");
    let p = Hist::reg(5, 0.0, 5.0).mean().to_root().unwrap().named("p");
    let path = std::env::temp_dir().join("oxiroot_traitapi_multi.root");
    FileWriter::create(&path)
        .add(&h1)
        .add(&h2)
        .add(&p)
        .compression(Compression::Stored)
        .write()
        .expect("write multi-object file");

    let f = FileReader::open(&path).expect("open");
    assert!(f.get::<Hist1D>("h1").is_ok());
    assert!(f.get::<Hist2D>("h2").is_ok());
    assert!(f.get::<Profile1D>("p").is_ok());
}
