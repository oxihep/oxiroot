//! Histograms, a `TTree` and an `RNTuple` in one file, through one `FileWriter`.

use crate::support::{NtupleObject, TreeObject};
use oxiroot::data::Column;
use oxiroot::file::Error;
use oxiroot::file::Parameter;
use oxiroot::ntuple::NtupleReader;
use oxiroot::ntuple::raw::Field;
use oxiroot::prelude::*;
use oxiroot::tree::TreeReader;
use oxiroot::tree::raw::Branch;

fn hist() -> Hist1D {
    let mut h = Hist::reg(4, 0.0, 4.0).double().named("pt");
    h.fill(0.5);
    h.fill(2.5);

    h.to_root().unwrap()
}

#[test]
fn histograms_trees_and_rntuples_share_a_file() {
    let path = std::env::temp_dir().join("oxiroot_mixed_all.root");
    FileWriter::create(&path)
        .add(&hist())
        .add(TreeObject::new(
            "Events",
            vec![
                Branch::new("energy", vec![10.5, 20.1, 5.0]),
                Branch::new("hits", vec![vec![1], vec![], vec![2, 3]]).counted(),
            ],
        ))
        .add(NtupleObject::new(
            "ntuple",
            vec![Field::new("x", vec![1, 2])],
        ))
        .dir("cal", |d| {
            d.add(&Parameter::f64(1.5).named("gain"))
                .add(TreeObject::new(
                    "Pedestals",
                    vec![Branch::new("adc", vec![3, 4])],
                ))
        })
        .write()
        .unwrap();

    let f = FileReader::open(&path).unwrap();
    assert_eq!(f.get::<Hist1D>("pt").unwrap(), hist());
    let tree = TreeReader::open(&f, "Events").unwrap();
    assert_eq!(
        tree.branch("energy").values().unwrap(),
        Column::F64(vec![10.5, 20.1, 5.0])
    );
    assert_eq!(
        tree.branch("hits").values().unwrap(),
        Column::from(vec![vec![1], vec![], vec![2, 3]])
    );
    assert_eq!(
        NtupleReader::open(&f, "ntuple")
            .unwrap()
            .field("x")
            .values()
            .unwrap(),
        Column::I32(vec![1, 2])
    );
    assert_eq!(
        f.get::<Parameter>("cal/gain").unwrap().value().as_f64(),
        1.5
    );
    let ped = TreeReader::open(&f, "cal/Pedestals").unwrap();
    assert_eq!(ped.branch("adc").values().unwrap(), Column::I32(vec![3, 4]));

    // One streamer-info record describes all of it.
    let registry = f.streamer_registry().unwrap();
    for class in [
        "TH1D",
        "TTree",
        "TBranch",
        "ROOT::RNTuple",
        "TParameter<double>",
    ] {
        assert!(registry.get(class).is_some(), "{class} is described");
    }
}

#[test]
fn a_tree_and_a_histogram_cannot_share_a_name() {
    let err = FileWriter::create("clash.root")
        .add(&hist())
        .add(TreeObject::new("pt", vec![Branch::new("x", vec![1])]))
        .compression(Compression::Stored)
        .to_bytes();
    assert!(matches!(err, Err(Error::DuplicateName { .. })), "{err:?}");
}

#[test]
fn a_tree_can_be_appended_to_a_histogram_file() {
    let path = std::env::temp_dir().join("oxiroot_mixed_append_tree.root");
    FileWriter::create(&path)
        .add(&hist())
        .compression(Compression::Stored)
        .write()
        .unwrap();
    oxiroot::FileUpdater::open(&path)
        .unwrap()
        .add(TreeObject::new(
            "T",
            vec![Branch::new("flag", vec![1_u8, 0])],
        ))
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let f = FileReader::open(&path).unwrap();
    assert_eq!(f.get::<Hist1D>("pt").unwrap(), hist());
    assert_eq!(
        TreeReader::open(&f, "T")
            .unwrap()
            .branch("flag")
            .values()
            .unwrap(),
        Column::U8(vec![1, 0])
    );
}

/// A `TTree` is more than its key — its baskets sit elsewhere in the file — so
/// compacting a file that holds one is refused, with the alternative named,
/// rather than writing a file whose tree has lost its data.
#[test]
fn compacting_a_file_that_holds_a_tree_is_refused() {
    let path = std::env::temp_dir().join("oxiroot_compact_with_tree.root");
    FileWriter::create(&path)
        .add(TreeObject::new("T", vec![Branch::new("x", vec![1, 2, 3])]))
        .compression(Compression::Stored)
        .write()
        .expect("write");

    let err = oxiroot::FileUpdater::open(&path)
        .expect("open")
        .compact()
        .compression(Compression::Stored)
        .write()
        .expect_err("compacting a tree file must be refused");
    let message = err.to_string();
    assert!(
        message.contains("TTree") && message.contains("hadd"),
        "{err}"
    );

    // The file is untouched: the tree still reads.
    let file = FileReader::open(&path).expect("reopen");
    let tree = TreeReader::open(&file, "T").expect("tree");
    assert_eq!(tree.entries(), 3);
}

/// Objects built in memory and written together: a histogram, a `TTree` and
/// an `RNTuple`, each added to the file the same way.
#[test]
fn named_batches_and_a_histogram_are_built_then_written_together() {
    use oxiroot::data::NamedBatch;

    let batch = |from: i32| {
        Batch::new()
            .column("id", (from..from + 3).collect::<Vec<i32>>())
            .unwrap()
    };
    let mut tree = NamedBatch::new("tree", Format::Tree, batch(0));
    tree.append(batch(3)).unwrap();
    let ntuple = NamedBatch::new("ntuple", Format::Ntuple, batch(10));
    let pt = hist();

    let bytes = FileWriter::create("built.root")
        .add(&pt)
        .add(&tree)
        .add(&ntuple)
        .to_bytes()
        .unwrap();
    let file = FileReader::from_bytes(bytes).unwrap();
    assert_eq!(file.get::<Hist1D>("pt").unwrap().contents(), pt.contents());
    let read = |name| {
        let ds = Dataset::open(&file, name).unwrap();
        ds.read(["id"])
            .unwrap()
            .values::<i32>("id")
            .unwrap()
            .to_vec()
    };
    assert_eq!(read("tree"), [0, 1, 2, 3, 4, 5]);
    assert_eq!(read("ntuple"), [10, 11, 12]);
}

/// A dataset streamed in several clusters, with the histogram filled along
/// the way stored in the same file: one pass, nothing held but a cluster.
#[test]
fn a_streamed_dataset_and_a_histogram_share_a_file() {
    for format in [Format::Tree, Format::Ntuple] {
        let sink = std::io::Cursor::new(Vec::new());
        let mut out = DatasetWriter::new(sink, "stream.root", "events", format).cluster_bytes(1);
        let mut pt = Hist::reg(4, 0.0, 4.0).double().named("pt");
        for chunk in 0..3 {
            let values: Vec<f64> = (0..4)
                .map(|i| f64::from(i) + 0.5 + f64::from(chunk))
                .collect();
            values.iter().for_each(|&v| pt.fill(v));
            out.append(Batch::new().column("pt", values).unwrap())
                .unwrap();
        }
        let pt = pt.to_root().unwrap();
        out.add(&pt);
        let file = FileReader::from_bytes(out.finish().unwrap().into_inner()).unwrap();

        let events = Dataset::open(&file, "events").unwrap();
        assert_eq!(events.entries(), 12, "{format:?}");
        assert!(events.batches(["pt"]).count() > 1, "{format:?}");
        assert_eq!(file.get::<Hist1D>("pt").unwrap().contents(), pt.contents());
    }
}
