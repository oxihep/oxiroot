//! Float-precision histogram write (`TH1F`/`TH2F`/`TH3F`): write, then read back
//! through our own reader. The files in /tmp are also checked by uproot/ROOT C++
//! in the interop job.

use oxiroot_hist::ToRoot;
use oxiroot_hist::{BinContentType, Hist, Hist1D, Hist2D, Hist3D};
use oxiroot_io_core::{Compression, FileReader};

#[test]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "small whole fill counts"
)]
fn th1f_write_read_round_trips() {
    let mut h = Hist::reg(4, 0.0, 4.0).double().named("h1").titled("float");
    for (i, &n) in [1.0, 2.0, 3.0, 4.0].iter().enumerate() {
        for _ in 0..(n as usize) {
            h.fill(i as f64 + 0.5);
        }
    }
    let h = h.to_root().unwrap();
    let out = std::path::PathBuf::from("/tmp/oxiroot_th1f.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h.clone().with_bin_content_type(BinContentType::F32))
        .compression(Compression::Stored)
        .write()
        .expect("write");

    let f = FileReader::open(&out).expect("reopen");
    assert_eq!(f.key("h1").expect("key").class_name, "TH1F");
    let back = f.get::<Hist1D>("h1").expect("read TH1F");
    assert_eq!(back.values(), h.values());
}

#[test]
fn th2f_write_read_round_trips() {
    let mut h = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .double()
        .named("h2")
        .titled("float2");
    h.fill(0.5, 0.5);
    h.fill(1.5, 1.5);
    h.fill(1.5, 1.5);
    let h = h.to_root().unwrap();
    let out = std::path::PathBuf::from("/tmp/oxiroot_th2f.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h.clone().with_bin_content_type(BinContentType::F32))
        .compression(Compression::Stored)
        .write()
        .expect("write");

    let f = FileReader::open(&out).expect("reopen");
    assert_eq!(f.key("h2").expect("key").class_name, "TH2F");
    let back = f.get::<Hist2D>("h2").expect("read TH2F");
    assert_eq!(back.values(), h.values());
}

#[test]
fn th3f_write_read_round_trips() {
    let mut h = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .double()
        .named("h3")
        .titled("float3");
    h.fill(0.5, 0.5, 0.5);
    h.fill(1.5, 1.5, 1.5);
    let h = h.to_root().unwrap();
    let out = std::path::PathBuf::from("/tmp/oxiroot_th3f.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&h.clone().with_bin_content_type(BinContentType::F32))
        .compression(Compression::Stored)
        .write()
        .expect("write");

    let f = FileReader::open(&out).expect("reopen");
    assert_eq!(f.key("h3").expect("key").class_name, "TH3F");
    let back = f.get::<Hist3D>("h3").expect("read TH3F");
    assert_eq!(back.values(), h.values());
}
