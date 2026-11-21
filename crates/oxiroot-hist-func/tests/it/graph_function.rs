//! A `Func1D` and a graph's `GraphFunction` are the same ROOT record: converting one
//! into the other keeps the bytes, and a function attached to a graph survives a
//! file round trip as something `Func1D` can evaluate.

use oxiroot_hist::{Graph, GraphFunction};
use oxiroot_hist_func::Func1D;
use oxiroot_io_core::{Compression, WriteRoot};
use oxiroot_io_core::{FileReader, WBuffer};

fn fitted() -> Func1D {
    Func1D::new("fit", "[0]*exp(-0.5*((x-[1])/[2])^2) + [3]", -2.0..=6.0)
        .unwrap()
        .with_params(&[4.0, 1.5, 0.75, 0.25])
        .unwrap()
}

#[test]
fn graph_function_record_has_the_tf1_key_bytes() {
    let f = fitted();
    let mut w = WBuffer::new();
    f.to_graph_function().write_tf1_body(&mut w, 1, 100);
    assert_eq!(w.into_vec(), f.to_root_bytes());
}

#[test]
fn conversion_round_trips_in_memory() {
    let f = fitted();
    let record = f.to_graph_function();
    assert_eq!(record.formula, f.formula());
    assert_eq!(record.params, f.params());
    assert_eq!(record.xmin..=record.xmax, f.range());
    assert_eq!(Func1D::from_graph_function(record).unwrap(), f);
}

#[test]
fn attached_function_evaluates_after_a_file_round_trip() {
    let f = fitted();
    let g = Graph::new(vec![0.0, 1.0, 2.0, 3.0], vec![1.3, 4.1, 3.2, 1.0])
        .unwrap()
        .named("g")
        .with_function(f.to_graph_function());
    let path = std::env::temp_dir().join("oxiroot_hist_func_graph_function.root");
    oxiroot_io_core::FileWriter::create(&path)
        .add(&g)
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let back = FileReader::open(&path).unwrap().get::<Graph>("g").unwrap();
    assert_eq!(back.functions.len(), 1);
    let h = Func1D::from_graph_function(back.functions[0].clone()).unwrap();
    for x in [-1.0, 0.0, 1.5, 2.25, 5.0] {
        assert_eq!(h.eval(x), f.eval(x), "x = {x}");
    }
    assert_eq!(h.name(), "fit");
    assert_eq!(h.range(), -2.0..=6.0);
}

#[test]
fn a_bad_formula_is_an_error() {
    let bad = GraphFunction::new("bad", "[0]*(x", vec![1.0], 0.0, 1.0);
    assert!(Func1D::from_graph_function(bad).is_err());
}

#[cfg(feature = "fit")]
#[test]
fn to_model_works_for_a_function_with_a_free_text_title() {
    let mut record = GraphFunction::new("line", "[0]+[1]*x", vec![1.0, 2.0], 0.0, 2.0);
    record.title = "Linear fit".to_string(); // a legend title, not a formula
    let f = Func1D::from_graph_function(record).unwrap();
    let model = f.to_model();
    assert_eq!(model.params(), vec![1.0, 2.0]);
    assert_eq!(model.param_names().len(), 2);
    for x in [0.0, 0.5, 1.75] {
        assert_eq!(model.eval(x), f.eval(x));
    }
}

#[cfg(feature = "fit")]
#[test]
fn to_model_keeps_the_names_of_a_shortcut_formula() {
    let f = Func1D::new("g", "gaus", -5.0..=5.0)
        .unwrap()
        .with_params(&[2.0, 0.5, 1.5])
        .unwrap();
    let model = f.to_model();
    let expected = oxifit::Model::from_formula("g", "gaus").unwrap();
    assert_eq!(model.param_names(), expected.param_names());
    assert_eq!(model.eval(0.25), f.eval(0.25));
}

#[cfg(feature = "fit")]
#[test]
fn a_func2d_and_a_func3d_fit_their_histograms() {
    use oxifit::FitExt;
    use oxiroot_hist::{Hist, ToRoot};
    use oxiroot_hist_func::{Func2D, Func3D};

    // A ROOT 2-D histogram filled with a plane, fitted with a TF2-like model.
    let mut h = Hist::reg(10, 0.0, 10.0)
        .reg(8, 0.0, 8.0)
        .double()
        .to_root()
        .unwrap()
        .named("plane");
    for ix in 0..10 {
        for iy in 0..8 {
            let (x, y) = (f64::from(ix) + 0.5, f64::from(iy) + 0.5);
            h.fill_weight(x, y, 2.0 + 0.5 * x + 1.5 * y);
        }
    }
    let f2 = Func2D::new("plane", "[0]+[1]*x+[2]*y", 0.0..=10.0, 0.0..=8.0)
        .unwrap()
        .with_params(&[1.0, 1.0, 1.0])
        .unwrap();
    let model = f2.to_model();
    assert_eq!(model.dim(), 2);
    assert_eq!(model.eval_at(&[3.0, 4.0]), f2.eval(3.0, 4.0));
    let fit = h.fit(&model);
    assert!(fit.valid, "{fit:?}");
    for (got, want) in fit.params.iter().zip([2.0, 0.5, 1.5]) {
        assert!((got - want).abs() < 1e-6, "{fit:?}");
    }
    assert_eq!(fit.ndf, 10 * 8 - 3);

    // And a 3-D one.
    let mut h3 = Hist::reg(4, 0.0, 4.0)
        .reg(3, 0.0, 3.0)
        .reg(5, 0.0, 5.0)
        .double()
        .to_root()
        .unwrap()
        .named("cube");
    for ix in 0..4 {
        for iy in 0..3 {
            for iz in 0..5 {
                let (x, y, z) = (
                    f64::from(ix) + 0.5,
                    f64::from(iy) + 0.5,
                    f64::from(iz) + 0.5,
                );
                h3.fill_weight(x, y, z, 1.0 + x - y + 2.0 * z);
            }
        }
    }
    let f3 = Func3D::new(
        "cube",
        "[0]+[1]*x+[2]*y+[3]*z",
        0.0..=4.0,
        0.0..=3.0,
        0.0..=5.0,
    )
    .unwrap()
    .with_params(&[0.0, 0.5, 0.0, 1.0])
    .unwrap();
    let fit = h3.fit(&f3.to_model());
    assert!(fit.valid, "{fit:?}");
    for (got, want) in fit.params.iter().zip([1.0, 1.0, -1.0, 2.0]) {
        assert!((got - want).abs() < 1e-6, "{fit:?}");
    }
}
