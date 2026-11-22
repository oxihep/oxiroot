//! Time axes (`Axis::fTimeDisplay` / `fTimeFormat`), which ROOT draws as dates
//! and clock times rather than numbers.

use oxiroot_hist::ToRoot;
use oxiroot_hist::{Graph, Hist, Hist1D};
use oxiroot_io_core::FileWriter;
use oxiroot_io_core::{Compression, FileReader};

/// ROOT's own spelling: a `strftime` format, then the epoch after `%F`.
const FORMAT: &str = "%H:%M%F2024-01-01 00:00:00";

#[test]
fn a_time_axis_survives_a_file() {
    let mut h = Hist::reg(24, 0.0, 86_400.0)
        .double()
        .named("rate")
        .titled("hits per hour");
    h.fill(3_600.0);
    // The time format is the ROOT axis's, set on the record.
    let mut h = h.to_root().unwrap();
    h.xaxis_mut().set_time_format(FORMAT);
    assert!(h.xaxis().is_time());

    // A graph carries its axes on its display frame, as ROOT stores them.
    let mut frame = Hist::reg(2, 0.0, 7_200.0)
        .double()
        .to_root()
        .unwrap()
        .with_bin_content_type(oxiroot_hist::BinContentType::F32)
        .named("Graph");
    frame.xaxis_mut().set_time_format(FORMAT);
    let g = Graph::new(vec![0.0, 3_600.0], vec![1.0, 2.0])
        .unwrap()
        .named("trend")
        .with_histogram(frame);

    let out = std::env::temp_dir().join("oxiroot_time_axis.root");
    FileWriter::create(&out)
        .add(&h)
        .add(&g)
        .compression(Compression::Stored)
        .write()
        .expect("write");

    let f = FileReader::open(&out).expect("reopen");
    let read = f.get::<Hist1D>("rate").expect("read hist");
    assert!(read.xaxis().is_time());
    assert_eq!(read.xaxis().time_format(), FORMAT);
    assert_eq!(read, h);

    let read = f.get::<Graph>("trend").expect("read graph");
    let frame = read.histogram().expect("display frame");
    assert!(frame.xaxis().is_time());
    assert_eq!(frame.xaxis().time_format(), FORMAT);
}

#[test]
fn an_ordinary_axis_stays_numeric() {
    let mut h = Hist::reg(4, 0.0, 4.0).double().named("plain");
    h.fill(1.5);
    let h = h.to_root().unwrap();
    assert!(!h.xaxis().is_time());
    assert_eq!(h.xaxis().time_format(), "");

    let out = std::env::temp_dir().join("oxiroot_plain_axis.root");
    FileWriter::create(&out)
        .add(&h)
        .compression(Compression::Stored)
        .write()
        .expect("write");
    let read = FileReader::open(&out)
        .unwrap()
        .get::<Hist1D>("plain")
        .unwrap();
    assert!(!read.xaxis().is_time());
    assert_eq!(read.xaxis().time_format(), "");

    // Setting and clearing leaves an axis as it started.
    let mut axis = read.xaxis().clone();
    axis.set_time_format("%H:%M");
    axis.clear_time_format();
    assert_eq!(&axis, read.xaxis());
}
