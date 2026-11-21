//! Collection objects: `HistStack` (a stack of histograms) and `GraphStack`
//! (several graphs). oxiroot reads the ROOT-C++-written `collections.root`
//! fixture — whose second list member uses a class back-reference — and
//! round-trips its own writes; ROOT C++ and uproot read oxiroot's output
//! (checked out of band).

use oxiroot_hist::ToRoot;
use std::path::PathBuf;

use oxiroot_hist::{Graph, GraphStack, Hist, HistStack};
use oxiroot_io_core::FileWriter;
use oxiroot_io_core::{Compression, FileReader};

fn fixture() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/collections.root"),
    )
    .expect("open fixture")
}

#[test]
fn reads_root_written_collections() {
    let f = fixture();

    let hs = f.get::<HistStack>("hs").unwrap();
    assert_eq!(hs.name(), "hs");
    assert_eq!(hs.title(), "my stack");
    assert_eq!(hs.members().len(), 2);
    assert_eq!(hs.members()[0].name(), "ha");
    assert_eq!(hs.members()[1].name(), "hb");
    // ha has bins 1,2 set; hb has bins 3,4 set (in-range bins only shown here).
    assert_eq!(hs.members()[0].values(), &[1.0, 2.0, 0.0, 0.0]);
    assert_eq!(hs.members()[1].values(), &[0.0, 0.0, 3.0, 4.0]);

    let mg = f.get::<GraphStack>("mg").unwrap();
    assert_eq!(mg.name(), "mg");
    assert_eq!(mg.title(), "my multigraph");
    assert_eq!(mg.members().len(), 2);
    assert_eq!(mg.members()[0].name(), "g1");
    assert_eq!(mg.members()[0].y(), vec![1.0, 2.0, 3.0]);
    assert_eq!(mg.members()[1].name(), "g2");
    assert_eq!(mg.members()[1].y(), vec![3.0, 2.0, 1.0]);
}

#[test]
fn round_trips_collections_through_oxiroot() {
    let mut ha = Hist::reg(4, 0.0, 4.0).double().named("ha").titled("a");
    ha.fill(0.5);
    let ha = ha.to_root().unwrap();
    let hb = Hist::reg(4, 0.0, 4.0)
        .double()
        .to_root()
        .unwrap()
        .named("hb")
        .titled("b");
    let stack = HistStack::new()
        .named("hs")
        .titled("my stack")
        .add(ha)
        .add(hb);

    let multi = GraphStack::new()
        .named("mg")
        .titled("my multigraph")
        .add(
            Graph::new(vec![0.0, 1.0, 2.0], vec![1.0, 2.0, 3.0])
                .unwrap()
                .named("g1"),
        )
        .add(
            Graph::new(vec![0.0, 1.0, 2.0], vec![3.0, 2.0, 1.0])
                .unwrap()
                .named("g2"),
        );

    let out = std::env::temp_dir().join("oxiroot_collections_rt.root");
    FileWriter::create(&out)
        .add(&stack)
        .add(&multi)
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let f = FileReader::open(&out).unwrap();
    let hs = f.get::<HistStack>("hs").unwrap();
    assert_eq!(hs.members().len(), 2);
    assert_eq!(hs.members()[0].name(), "ha");
    assert_eq!(hs.members()[0].values(), &[1.0, 0.0, 0.0, 0.0]); // the one fill
    let mg = f.get::<GraphStack>("mg").unwrap();
    assert_eq!(mg.members().len(), 2);
    assert_eq!(mg.members()[1].y(), vec![3.0, 2.0, 1.0]);
    let _ = std::fs::remove_file(&out);
}

/// ROOT names `TF1` in full for the first function in a buffer and by a class
/// back-reference afterwards, so the function of a multigraph's second member
/// refers to a class named inside the first. Both must be read.
#[test]
fn reads_the_functions_of_every_graph_in_a_root_multigraph() {
    let f = FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/multigraph_functions.root"),
    )
    .unwrap();
    let mg = f.get::<GraphStack>("mg").unwrap();
    let functions: Vec<Vec<&str>> = mg
        .members()
        .iter()
        .map(|g| g.functions().iter().map(|f| f.name.as_str()).collect())
        .collect();
    assert_eq!(functions, [vec!["lin"], vec!["sq"]]);
    assert_eq!(mg.members()[1].functions()[0].formula, "[p0]*x*x");
}

fn list_functions() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/list_functions.root")
}

/// The same across the members of a `TList`: the second graph's `TF1`, and its
/// `TList` of functions, are named by references into the first.
#[test]
fn reads_the_functions_of_every_graph_in_a_root_list() {
    let f = FileReader::open(list_functions()).unwrap();
    let list = f.get::<oxiroot_io_core::ObjList>("graphs").unwrap();
    let graphs: Vec<Graph> = list.items().unwrap();
    let functions: Vec<Vec<&str>> = graphs
        .iter()
        .map(|g| g.functions().iter().map(|f| f.name.as_str()).collect())
        .collect();
    assert_eq!(functions, [vec!["lin"], vec!["sq"]]);
}

/// A list read from a file is written back as it was read: its members refer
/// to each other by position, so they cannot be re-encoded one by one.
#[test]
fn a_root_list_is_written_back_byte_for_byte() {
    use oxiroot_io_core::{Compression, FileWriter, ObjList, WriteRoot};

    let f = FileReader::open(list_functions()).unwrap();
    let list = f.get::<ObjList>("graphs").unwrap();
    let out = std::env::temp_dir().join("oxiroot_list_back.root");
    FileWriter::create(&out)
        .add(&list)
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let back = FileReader::open(&out).unwrap();
    let (original, copy) = (f.key("graphs").unwrap(), back.key("graphs").unwrap());
    assert_eq!(copy.key_len, original.key_len);
    assert_eq!(
        back.get::<ObjList>("graphs").unwrap().to_root_bytes(),
        list.to_root_bytes()
    );
    let graphs: Vec<Graph> = back.get::<ObjList>("graphs").unwrap().items().unwrap();
    assert_eq!(graphs[1].functions()[0].name, "sq");
    std::fs::remove_file(out).unwrap();
}

/// What cannot be written correctly is refused: the list under another name
/// (its references count from the key), or inside another list.
#[test]
fn a_root_list_refuses_a_key_its_references_do_not_fit() {
    use oxiroot_io_core::{FileWriter, ListKind, ObjList};

    let f = FileReader::open(list_functions()).unwrap();
    let list = f.get::<ObjList>("graphs").unwrap();

    let renamed = list.clone().named("somewhere_else");
    let err = FileWriter::create("unused.root")
        .add(&renamed)
        .to_bytes()
        .unwrap_err();
    assert!(err.to_string().contains("cannot be renamed"), "{err}");

    let nested = ObjList::new(ListKind::List).named("outer").add(&list);
    let err = FileWriter::create("unused.root")
        .add(&nested)
        .to_bytes()
        .unwrap_err();
    assert!(err.to_string().contains("under its own key"), "{err}");
}

/// Members added to a list read from a file follow the ones it had.
#[test]
fn a_root_list_takes_new_members() {
    use oxiroot_io_core::{FileWriter, ObjList};

    let f = FileReader::open(list_functions()).unwrap();
    let extra = Graph::new(vec![1.0, 2.0], vec![3.0, 4.0])
        .unwrap()
        .named("c");
    let list = f.get::<ObjList>("graphs").unwrap().add(&extra);
    let bytes = FileWriter::create("unused.root")
        .add(&list)
        .to_bytes()
        .unwrap();

    let back = FileReader::from_bytes(bytes).unwrap();
    let graphs: Vec<Graph> = back.get::<ObjList>("graphs").unwrap().items().unwrap();
    let names: Vec<&str> = graphs.iter().map(Graph::name).collect();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(graphs[1].functions()[0].name, "sq");
}
