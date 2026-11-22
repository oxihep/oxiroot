//! Functions attached to a histogram (`fFunctions`): the curve `TH1::Fit`
//! leaves with it. Read from ROOT's `hist_functions.root`, and written back.

use std::path::PathBuf;

use oxiroot_hist::{GraphFunction, Hist, Hist1D, HistStack, ToRoot};
use oxiroot_io_core::{FileReader, FileWriter};

fn fixture() -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hist_functions.root"),
    )
    .expect("open fixture")
}

fn names(h: &Hist1D) -> Vec<&str> {
    h.functions().iter().map(|f| f.name.as_str()).collect()
}

/// Both functions are read: the second `TF1` is named by a reference to the
/// first.
#[test]
fn reads_the_functions_root_attached_to_a_histogram() {
    let h = fixture().get::<Hist1D>("h").unwrap();
    assert_eq!(h.values(), [1.0, 0.0, 2.0, 0.0]);
    assert_eq!(names(&h), ["line_h", "peak"]);
    let line = &h.functions()[0];
    assert_eq!(line.formula, "[p0]+[p1]*x");
    assert_eq!(line.params, [1.0, 0.5]);
    // ROOT stores `gaus` with named parameters; they are read by position.
    let peak = &h.functions()[1];
    assert_eq!(
        peak.formula,
        "[p0]*exp(-0.5*((x-[p1])/[p2])*((x-[p1])/[p2]))"
    );
    assert_eq!(peak.params, [3.0, 2.0, 0.5]);
}

/// And in a `THStack`, where the second histogram's function refers to a
/// class named inside the first.
#[test]
fn reads_the_functions_of_every_histogram_in_a_root_stack() {
    let stack = fixture().get::<HistStack>("hs").unwrap();
    let functions: Vec<Vec<&str>> = stack.members().iter().map(names).collect();
    assert_eq!(functions, [vec!["line_s1"], vec!["line_s2"]]);
    assert_eq!(stack.members()[1].functions()[0].params, [5.0, 0.5]);
}

/// A histogram keeps its functions through a write and a read, alone and in
/// a stack.
#[test]
fn a_histogram_is_written_with_its_functions() {
    let file = fixture();
    let h = file.get::<Hist1D>("h").unwrap();
    let stack = file.get::<HistStack>("hs").unwrap();
    let mut mine = Hist::reg(4, 0.0, 4.0).double().named("mine");
    mine.fill(1.5);
    let mine = mine
        .to_root()
        .unwrap()
        .with_function(GraphFunction::new("[0]*x", vec![2.5], 0.0, 4.0).named("fit"));

    let bytes = FileWriter::create("unused.root")
        .add(&h)
        .add(&stack)
        .add(&mine)
        .to_bytes()
        .unwrap();
    let back = FileReader::from_bytes(bytes).unwrap();
    assert_eq!(back.get::<Hist1D>("h").unwrap(), h);
    assert_eq!(back.get::<HistStack>("hs").unwrap(), stack);
    assert_eq!(back.get::<Hist1D>("mine").unwrap(), mine);
    assert_eq!(names(&back.get::<Hist1D>("mine").unwrap()), ["fit"]);
}

/// A histogram without functions is written as before.
#[test]
fn a_plain_histogram_has_no_functions() {
    let h = Hist::reg(4, 0.0, 4.0)
        .double()
        .named("h")
        .to_root()
        .unwrap();
    assert!(h.functions().is_empty());
}
