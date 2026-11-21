//! #3: write several histograms (a TH1D and a TH2D) into a single ROOT file,
//! then read each back through our own reader.

use oxiroot_hist::ToRoot;
use std::path::PathBuf;

use oxiroot_hist::{Hist, Hist1D, Hist2D};
use oxiroot_io_core::FileReader;
use oxiroot_io_core::FileWriter;

#[test]
fn writes_multiple_histograms_into_one_file() {
    let mut h1 = Hist::reg(4, 0.0, 4.0).double().named("hx").titled("1-D");
    for x in [0.5, 1.5, 1.5, 3.5] {
        h1.fill(x);
    }
    let h1 = h1.to_root().unwrap();
    let mut h2 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .double()
        .named("hxy")
        .titled("2-D");
    h2.fill(0.5, 0.5);
    h2.fill(1.5, 1.5);
    h2.fill(1.5, 1.5);
    let h2 = h2.to_root().unwrap();

    let out = PathBuf::from("/tmp/rootrs_multi_hist.root");
    FileWriter::create(&out)
        .add(&h1)
        .add(&h2)
        .compression(oxiroot_io_core::Compression::Stored)
        .write()
        .expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let keys: Vec<(&str, &str)> = f
        .keys()
        .iter()
        .map(|k| (k.name.as_str(), k.class_name.as_str()))
        .collect();
    assert_eq!(keys, vec![("hx", "TH1D"), ("hxy", "TH2D")]);

    assert_eq!(f.get::<Hist1D>("hx").expect("read hx"), h1);
    assert_eq!(f.get::<Hist2D>("hxy").expect("read hxy"), h2);
}
