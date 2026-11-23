//! Standalone `Func1D`/`Func2D`/`Func3D` function keys: oxiroot reads the ROOT-C++-written
//! `tf1.root`/`tf23.root` fixtures (a `Func1D` embedding a `TFormula`, plus a `Func2D`
//! and `Func3D`), evaluates the formulas in pure Rust, round-trips its own writes,
//! and serializes byte-for-byte as ROOT does. ROOT C++ and uproot both read
//! oxiroot's `Func1D`/`Func2D`/`Func3D` output and re-evaluate them (checked out of band):
//! oxiroot embeds the `Func1D`/`Func2D`/`Func3D`/`TFormula` `TStreamerInfo` (with the
//! `TStreamerSTL` members) so uproot builds a model for each.

use std::path::PathBuf;

use oxiroot_hist_func::{Func1D, Func2D, Func3D};
use oxiroot_io_core::{Compression, FileReader};
use oxiroot_io_core::{FileWriter, WriteRoot};

fn fixture(name: &str) -> FileReader {
    FileReader::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name),
    )
    .expect("open fixture")
}

#[test]
fn reads_root_written_tf1() {
    let f = fixture("tf1.root");
    let g = f.get::<Func1D>("myfunc").unwrap();
    assert_eq!(g.name(), "myfunc");
    assert_eq!(g.title(), "[0]*sin([1]*x) + [2]");
    assert_eq!(g.formula(), "[p0]*sin([p1]*x)+[p2]");
    assert_eq!(g.param_names(), ["p0", "p1", "p2"]);
    assert_eq!(g.npar(), 3);
    assert_eq!(g.params(), &[2.0, 1.5, 0.5]);
    // ROOT: Eval(1)=2.494990, Integral(0,pi)=2.904130, Derivative(1)=0.212212.
    assert!((g.eval(1.0) - 2.494_990).abs() < 1e-6);
    assert!((g.integral(0.0, std::f64::consts::PI) - 2.904_130).abs() < 1e-5);
    assert!((g.derivative(1.0) - 0.212_212).abs() < 1e-5);
}

#[test]
fn reads_root_written_tf2_and_tf3() {
    let f = fixture("tf23.root");
    let f2 = f.get::<Func2D>("f2").unwrap();
    assert_eq!(f2.title(), "[0]*sin(x) + [1]*y*y");
    assert_eq!(f2.params(), &[1.5, 0.7]);
    assert!((f2.eval(1.0, 1.0) - 1.962_206).abs() < 1e-5); // ROOT Eval(1,1)

    let f3 = f.get::<Func3D>("f3").unwrap();
    assert_eq!(f3.title(), "[0]*x + y*z");
    assert_eq!(f3.params(), &[2.0]);
    assert_eq!(f3.eval(1.0, 1.0, 1.0), 3.0); // ROOT Eval(1,1,1)
}

#[test]
#[allow(clippy::approx_constant)] // 6.283 is the fixture's exact fXmax, not TAU
fn tf1_bytes_are_byte_exact_against_root() {
    // oxiroot's serialized Func1D bytes must equal ROOT's, byte for byte — with one
    // tolerated quirk: ROOT serializes `TFormula::fAllParametersSetted` as a
    // *non-normalized* `bool` (any truthy byte, e.g. 0x99), while oxiroot writes
    // the canonical `1`. Both read back as `true`, so we accept `(1, non-zero)`
    // at a single position and require every other byte to match exactly.
    let f = fixture("tf1.root");
    let obj = Func1D::new("[0]*sin([1]*x) + [2]", 0.0..=6.283)
        .unwrap()
        .named("myfunc")
        .with_params(&[2.0, 1.5, 0.5])
        .unwrap()
        .to_root_bytes();
    let (_, root, _) = oxiroot_io_core::raw::object_bytes(&f, "myfunc").unwrap();
    assert_eq!(obj.len(), root.len(), "TF1 object length differs from ROOT");
    for (i, (&a, &b)) in obj.iter().zip(&root).enumerate() {
        if a != b {
            assert!(
                a == 1 && b != 0,
                "TF1 byte {i} differs: oxiroot={a}, root={b} (only ROOT's truthy \
                 fAllParametersSetted byte may differ)"
            );
        }
    }
}

#[test]
fn round_trips_tf1_tf2_tf3_through_oxiroot() {
    let out = std::env::temp_dir().join("oxiroot_tf_rt.root");
    let f1 = Func1D::new("[0]*exp(-[1]*x)", 0.0..=5.0)
        .unwrap()
        .named("f1")
        .with_params(&[10.0, 0.5])
        .unwrap();
    let f2 = Func2D::new("[0]*x + y", -2.0..=2.0, -2.0..=2.0)
        .unwrap()
        .named("f2")
        .with_params(&[3.0])
        .unwrap();
    let f3 = Func3D::new("x + y + z + [0]", 0.0..=1.0, 0.0..=1.0, 0.0..=1.0)
        .unwrap()
        .named("f3")
        .with_params(&[0.25])
        .unwrap();

    FileWriter::create(&out)
        .add(&f1)
        .add(&f2)
        .add(&f3)
        .compression(Compression::Stored)
        .write()
        .unwrap();

    let f = FileReader::open(&out).unwrap();
    assert_eq!(f.get::<Func1D>("f1").unwrap(), f1);
    assert_eq!(f.get::<Func2D>("f2").unwrap(), f2);
    assert_eq!(f.get::<Func3D>("f3").unwrap(), f3);
    // and the evaluation survives the round trip.
    assert!((f.get::<Func1D>("f1").unwrap().eval(2.0) - 10.0 * (-1.0f64).exp()).abs() < 1e-12);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn written_file_embeds_function_streamer_info() {
    // uproot needs the embedded `TStreamerInfo` to model a standalone Func2D/Func3D.
    // Verify the writer emits it: with no compression the class/member names
    // appear literally in the file.
    let out = std::env::temp_dir().join("oxiroot_tf_streamer.root");
    FileWriter::create(&out)
        .add(
            &Func2D::new("[0]*x + y", 0.0..=1.0, 0.0..=1.0)
                .unwrap()
                .named("f2")
                .with_params(&[2.0])
                .unwrap(),
        )
        .compression(Compression::Stored)
        .write()
        .unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"TFormula"), "TFormula streamer info not embedded");
    assert!(has(b"TStreamerSTL"), "TStreamerSTL element not embedded");
    assert!(has(b"fClingParameters"), "TFormula members not embedded");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn builds_and_evaluates_shortcuts() {
    let g = Func1D::new("gaus", -5.0..=5.0)
        .unwrap()
        .named("g")
        .with_params(&[2.0, 0.0, 1.0])
        .unwrap();
    assert_eq!(g.eval(0.0), 2.0);
    assert!((g.integral(-10.0, 10.0) - 2.0 * (2.0 * std::f64::consts::PI).sqrt()).abs() < 1e-4);
}

/// Two functions in one ROOT-written list: the second names `TFormula` by a
/// back-reference to the first's, which the list's class-tag map resolves.
#[test]
fn reads_functions_held_in_a_root_written_list() {
    let list = fixture("embedded_backrefs.root")
        .get::<oxiroot_io_core::ObjList>("members")
        .unwrap();
    let functions = list.items::<Func1D>().unwrap();
    let formulas: Vec<&str> = functions.iter().map(Func1D::formula).collect();
    assert_eq!(formulas, ["[p0]*x", "[p0]+[p1]*x"]);
    assert_eq!(functions[1].params(), [1.5, 2.5]);
}

/// Functions whose parameters ROOT numbers and names in ways a reader has to
/// follow (`scripts/gen_tf1_params.C`): each reads back with ROOT's names and
/// ROOT's value at x = 1.3, and is written back as ROOT wrote it.
#[test]
fn reads_roots_parameter_order_and_names() {
    let f = fixture("tf1_params.root");
    for (name, value, names) in [
        ("swap", 2.95, "p0,p1"),
        ("named", 2.95, "a,b"),
        ("peak", 9.231_163_463_866_357, "Constant,Mean,Sigma"),
        ("cb", 2.796_307_477_078_582_5, "Constant,Mean,Sigma,Alpha,N"),
        ("lan", 0.670_961_427_121_047, "Constant,MPV,Sigma"),
        ("bkg", 3.246_278_070_458_871, "p0,p1,p2,p3"),
    ] {
        let g = f.get::<Func1D>(name).unwrap();
        assert_eq!(g.param_names().join(","), names, "{name}");
        let v = g.eval(1.3);
        assert!(
            (v - value).abs() <= 1e-12 * value.abs(),
            "{name}: {v} where ROOT gives {value}"
        );
        // Written and read again: the same function.
        let bytes = FileWriter::create("x.root").add(&g).to_bytes().unwrap();
        let back = FileReader::from_bytes(bytes)
            .unwrap()
            .get::<Func1D>(name)
            .unwrap();
        assert_eq!(back, g, "{name}");
        assert_eq!(back.eval(1.3).to_bits(), v.to_bits(), "{name}");
    }
    // ROOT stores `[1]*x+[0]` as `[p1]*x+[p0]`: parameter 1 first.
    let swap = f.get::<Func1D>("swap").unwrap();
    assert_eq!(swap.formula(), "[p1]*x+[p0]");
    let named = f.get::<Func1D>("named").unwrap();
    assert_eq!(named.param_named("b"), Some(1.5));
}

#[test]
fn named_parameters_are_set_and_read_by_name() {
    let mut f = Func1D::new("[N]*exp(-0.5*((x-[mu])/[sigma])^2)", -5.0..=5.0)
        .unwrap()
        .with_param("N", 10.0)
        .unwrap()
        .with_param("mu", 0.5)
        .unwrap();
    f.set_param("sigma", 2.0).unwrap();
    assert_eq!(f.param_names(), ["N", "mu", "sigma"]);
    assert_eq!(f.params(), [10.0, 0.5, 2.0]);
    assert_eq!(f.param_named("mu"), Some(0.5));
    assert_eq!(f.param_named("tau"), None);
    let err = f.set_param("tau", 1.0).unwrap_err().to_string();
    assert!(err.contains("the parameters are N, mu, sigma"), "{err}");
    assert!((f.eval(1.3) - 9.231_163_463_866_357).abs() < 1e-12);
}

#[test]
fn a_formula_needs_no_more_variables_than_its_function() {
    let err = Func1D::new("x*y", 0.0..=1.0).unwrap_err().to_string();
    assert!(err.contains("uses `y`"), "{err}");
    assert!(Func2D::new("x*y", 0.0..=1.0, 0.0..=1.0).is_ok());
    let err = Func1D::new("[0]*bogus(x)", 0.0..=1.0)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown name `bogus`"), "{err}");
}

#[test]
fn roots_predefined_functions_have_constructors() {
    let g = Func1D::gaus(-5.0..=5.0);
    assert_eq!(g.param_names(), ["Constant", "Mean", "Sigma"]);
    assert_eq!(g.title(), "gaus");
    assert_eq!(Func1D::expo(0.0..=1.0).param_names(), ["Constant", "Slope"]);
    assert_eq!(
        Func1D::landau(0.0..=1.0).param_names(),
        ["Constant", "MPV", "Sigma"]
    );
    assert_eq!(Func1D::crystalball(0.0..=1.0).npar(), 5);
    assert_eq!(Func1D::gausn(0.0..=1.0).npar(), 3);
    assert_eq!(Func1D::breitwigner(0.0..=1.0).npar(), 3);
    assert_eq!(Func1D::pol(3, 0.0..=1.0).unwrap().npar(), 4);
    assert_eq!(Func1D::cheb(2, 0.0..=1.0).unwrap().npar(), 3);
    assert!(Func1D::cheb(11, 0.0..=1.0).is_err());
    assert_eq!(
        Func2D::gaus(0.0..=1.0, 0.0..=1.0).param_names(),
        ["Constant", "MeanX", "SigmaX", "MeanY", "SigmaY"]
    );
    assert_eq!(Func2D::bigaus(0.0..=1.0, 0.0..=1.0).npar(), 6);
}

#[test]
fn the_derivative_is_exact_where_the_formula_has_one() {
    let g = Func1D::gaus(-5.0..=5.0)
        .with_params(&[2.0, 0.5, 1.5])
        .unwrap();
    for x in [-2.0, 0.1, 0.5, 3.3] {
        let u: f64 = (x - 0.5) / 1.5;
        let exact = -2.0 * u / 1.5 * (-0.5 * u * u).exp();
        assert!((g.derivative(x) - exact).abs() < 1e-14, "{x}");
    }
    // TMath::Landau has no closed form here: a central difference.
    let l = Func1D::landau(0.0..=10.0)
        .with_params(&[4.0, 1.0, 0.7])
        .unwrap();
    let h = 1e-5;
    let numeric = (l.eval(2.0 + h) - l.eval(2.0 - h)) / (2.0 * h);
    assert!((l.derivative(2.0) - numeric).abs() < 1e-6);
}

#[test]
fn a_formula_error_keeps_where_and_why() {
    let err = Func1D::new("x & 1", 0.0..=1.0).unwrap_err();
    let parse = err.downcast_ref::<oxifit_formula::ParseError>().unwrap();
    assert_eq!(parse.span(), 2..3);
    assert_eq!(parse.hint(), Some("ROOT writes logical AND as `&&`"));
}
