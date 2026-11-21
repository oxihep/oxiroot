//! Integer-precision histogram write (`TH1C`/`TH1S`/`TH1I` and 2D/3D): write,
//! then read back through our own reader. The /tmp files are also checked by
//! uproot/ROOT C++ when run by hand.

use oxiroot_hist::ToRoot;
use oxiroot_hist::{BinContentType, Hist, Hist1D, Hist2D, Hist3D};
use oxiroot_io_core::{Compression, FileReader};

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "small whole fill counts"
)]
fn filled_th1() -> Hist1D {
    let mut h = Hist::reg(4, 0.0, 4.0).double().named("h").titled("int");
    for (i, &n) in [1.0, 2.0, 3.0, 4.0].iter().enumerate() {
        for _ in 0..(n as usize) {
            h.fill(i as f64 + 0.5);
        }
    }

    h.to_root().unwrap()
}

fn check_th1(out: &str, cls: &str, h: &Hist1D) {
    let f = FileReader::open(out).expect("reopen");
    assert_eq!(f.key("h").expect("key").class_name, cls);
    assert_eq!(
        f.get::<Hist1D>("h").expect("read back").values(),
        h.values()
    );
}

#[test]
fn th1_integer_variants_round_trip() {
    let h = filled_th1();
    oxiroot_io_core::FileWriter::create("/tmp/oxiroot_th1c.root")
        .add(&h.clone().with_bin_content_type(BinContentType::I8))
        .compression(Compression::Stored)
        .write()
        .expect("write C");
    check_th1("/tmp/oxiroot_th1c.root", "TH1C", &h);
    oxiroot_io_core::FileWriter::create("/tmp/oxiroot_th1s.root")
        .add(&h.clone().with_bin_content_type(BinContentType::I16))
        .compression(Compression::Stored)
        .write()
        .expect("write S");
    check_th1("/tmp/oxiroot_th1s.root", "TH1S", &h);
    oxiroot_io_core::FileWriter::create("/tmp/oxiroot_th1i.root")
        .add(&h.clone().with_bin_content_type(BinContentType::I32))
        .compression(Compression::Stored)
        .write()
        .expect("write I");
    check_th1("/tmp/oxiroot_th1i.root", "TH1I", &h);
    oxiroot_io_core::FileWriter::create("/tmp/oxiroot_th1l.root")
        .add(&h.clone().with_bin_content_type(BinContentType::I64))
        .compression(Compression::Stored)
        .write()
        .expect("write L");
    check_th1("/tmp/oxiroot_th1l.root", "TH1L", &h);
}

#[test]
fn th2i_th3i_round_trip() {
    let mut h2 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .double()
        .named("h2")
        .titled("i2");
    h2.fill(0.5, 0.5);
    h2.fill(1.5, 1.5);
    let h2 = h2.to_root().unwrap();
    let out = std::path::PathBuf::from("/tmp/oxiroot_th2i.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h2.clone().with_bin_content_type(BinContentType::I32))
        .compression(Compression::Stored)
        .write()
        .expect("write");
    let f = FileReader::open(&out).expect("reopen");
    assert_eq!(f.key("h2").expect("key").class_name, "TH2I");
    assert_eq!(f.get::<Hist2D>("h2").unwrap().values(), h2.values());

    let mut h3 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .double()
        .named("h3")
        .titled("i3");
    h3.fill(0.5, 0.5, 0.5);
    let h3 = h3.to_root().unwrap();
    let out = std::path::PathBuf::from("/tmp/oxiroot_th3i.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h3.clone().with_bin_content_type(BinContentType::I32))
        .compression(Compression::Stored)
        .write()
        .expect("write");
    let f = FileReader::open(&out).expect("reopen");
    assert_eq!(f.key("h3").expect("key").class_name, "TH3I");
    assert_eq!(f.get::<Hist3D>("h3").unwrap().values(), h3.values());
}
