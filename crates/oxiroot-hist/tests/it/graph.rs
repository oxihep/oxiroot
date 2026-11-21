//! Graph / `TGraphErrors` / `TGraphAsymmErrors`: read ROOT fixtures, self
//! round-trip, and build from scratch. (Cross-checked against compiled ROOT C++
//! and uproot, which read the oxiroot-written files with the right classes and
//! values.)

use oxiroot_hist::ToRoot;
use std::path::PathBuf;

use oxiroot_hist::{Graph, GraphErrors, Hist};
use oxiroot_io_core::{Compression, FileReader};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

#[test]
fn reads_root_written_graphs() {
    let f = FileReader::open(fixture("graphs.root")).expect("open");

    let g = f.get::<Graph>("g").expect("read g");
    assert_eq!(g.class_name(), "TGraph");
    assert_eq!(g.title, "plain");
    assert_eq!(g.x(), vec![1.0, 2.0, 3.0, 4.0]);
    assert_eq!(g.y(), vec![10.0, 20.0, 30.0, 40.0]);
    assert_eq!(*g.errors(), GraphErrors::Plain);

    let ge = f.get::<Graph>("ge").expect("read ge");
    assert_eq!(ge.class_name(), "TGraphErrors");
    assert_eq!(
        *ge.errors(),
        GraphErrors::Symmetric {
            ex: vec![0.1, 0.2, 0.3, 0.4],
            ey: vec![1.0, 2.0, 3.0, 4.0],
        }
    );

    let gae = f.get::<Graph>("gae").expect("read gae");
    assert_eq!(gae.class_name(), "TGraphAsymmErrors");
    assert_eq!(
        *gae.errors(),
        GraphErrors::Asymmetric {
            ex_low: vec![0.1, 0.1, 0.1, 0.1],
            ex_high: vec![0.2, 0.2, 0.2, 0.2],
            ey_low: vec![1.0, 1.0, 1.0, 1.0],
            ey_high: vec![2.0, 2.0, 2.0, 2.0],
        }
    );
}

#[test]
fn graphs_round_trip() {
    let f = FileReader::open(fixture("graphs.root")).expect("open");
    for name in ["g", "ge", "gae"] {
        let g = f.get::<Graph>(name).unwrap();
        let out = std::env::temp_dir().join(format!("oxiroot_graph_{name}.root"));
        oxiroot_io_core::FileWriter::create(&out)
            .add(&g)
            .compression(Compression::Stored)
            .write()
            .expect("write");
        let back = FileReader::open(&out).unwrap().get::<Graph>(name).unwrap();
        assert_eq!(back, g, "{name} changed across round-trip");
    }
}

#[test]
fn graphs_build_from_scratch() {
    let plain = Graph::new(vec![0.0, 1.0, 2.0], vec![1.0, 4.0, 9.0])
        .unwrap()
        .named("p")
        .titled("plain");
    let sym = Graph::new(vec![0.0, 1.0], vec![2.0, 3.0])
        .and_then(|g| {
            g.with_errors(GraphErrors::Symmetric {
                ex: vec![0.1, 0.1],
                ey: vec![0.5, 0.5],
            })
        })
        .unwrap()
        .named("s")
        .titled("sym");
    let asym = Graph::new(vec![0.0, 1.0], vec![2.0, 3.0])
        .and_then(|g| {
            g.with_errors(GraphErrors::Asymmetric {
                ex_low: vec![0.1, 0.2],
                ex_high: vec![0.3, 0.4],
                ey_low: vec![1.0, 1.0],
                ey_high: vec![2.0, 2.0],
            })
        })
        .unwrap()
        .named("a")
        .titled("asym");

    for g in [&plain, &sym, &asym] {
        let out = std::env::temp_dir().join(format!("oxiroot_graph_scratch_{}.root", g.name));
        oxiroot_io_core::FileWriter::create(&out)
            .add(g)
            .compression(Compression::Zstd(3))
            .write()
            .expect("write");
        let back = FileReader::open(&out)
            .unwrap()
            .get::<Graph>(&g.name)
            .unwrap();
        assert_eq!(back, *g);
    }

    assert_eq!(plain.class_name(), "TGraph");
    assert_eq!(sym.class_name(), "TGraphErrors");
    assert_eq!(asym.class_name(), "TGraphAsymmErrors");
}

/// Zero-point graphs are a boundary worth pinning: the writer must still emit a
/// valid (empty) `fNpoints`/`fX`/`fY`, and the reader must round-trip them.
#[test]
fn empty_graphs_round_trip() {
    let plain = Graph::new(vec![], vec![])
        .unwrap()
        .named("e0")
        .titled("empty");
    let sym = Graph::new(vec![], vec![])
        .and_then(|g| {
            g.with_errors(GraphErrors::Symmetric {
                ex: vec![],
                ey: vec![],
            })
        })
        .unwrap()
        .named("e1")
        .titled("empty");
    for g in [&plain, &sym] {
        assert_eq!(g.len(), 0);
        assert!(g.is_empty());
        let out = std::env::temp_dir().join(format!("oxiroot_graph_empty_{}.root", g.name));
        oxiroot_io_core::FileWriter::create(&out)
            .add(g)
            .compression(Compression::Stored)
            .write()
            .expect("write");
        let back = FileReader::open(&out)
            .unwrap()
            .get::<Graph>(&g.name)
            .unwrap();
        assert_eq!(back, *g);
    }
}

/// A graph ROOT drew before writing carries a real `fHistogram` display frame;
/// we read it back as a `TH1F` (previously this member was skipped).
#[test]
fn reads_root_graph_histogram() {
    let f = FileReader::open(fixture("graph_hist.root")).expect("open");
    let g = f.get::<Graph>("g").expect("read g");
    let h = g
        .histogram
        .expect("graph carries an fHistogram display frame");
    assert_eq!(h.class_name(), "TH1F"); // ROOT's fHistogram is a TH1F
    assert_eq!(h.xaxis().nbins(), 100); // ROOT's default frame binning
    // The plain graphs in graphs.root were never drawn, so they have no frame.
    let g0 = FileReader::open(fixture("graphs.root"))
        .unwrap()
        .get::<Graph>("g")
        .unwrap();
    assert!(g0.histogram.is_none());
}

/// A user-attached display frame round-trips (and is persisted as a `TH1F`).
#[test]
fn graph_histogram_round_trips() {
    let frame = Hist::reg(20, -5.0, 5.0)
        .double()
        .to_root()
        .unwrap()
        .named("Graph")
        .titled("frame;x;y");
    let g = Graph::new(vec![1.0, 2.0], vec![3.0, 4.0])
        .and_then(|g| {
            g.with_errors(GraphErrors::Symmetric {
                ex: vec![0.1, 0.1],
                ey: vec![0.5, 0.5],
            })
        })
        .unwrap()
        .named("g")
        .titled("framed")
        .with_histogram(frame);
    assert!(g.histogram.is_some());

    let out = std::env::temp_dir().join("oxiroot_graph_hist_rt.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&g)
        .compression(Compression::Zstd(3))
        .write()
        .expect("write");
    let back = FileReader::open(&out).unwrap().get::<Graph>("g").unwrap();
    assert_eq!(back, g); // exact round-trip, including the TH1F frame
    assert_eq!(back.histogram.unwrap().xaxis().nbins(), 20);
    let _ = std::fs::remove_file(&out);
}
