//! State that is set through a builder is read back through its accessor,
//! and survives a file: the options of an efficiency, a profile and a
//! multi-error graph, and an axis's name, title and time format.

use oxiroot_hist::{
    Axis, Efficiency, ErrorMode, Hist, Mergeable, MultiErrorGraph, Profile1D, Profile2D, Profile3D,
    StatisticOption, SumErrorsMode, ToRoot,
};
use oxiroot_io_core::{FileReader, FileWriter, ReadRoot, WriteRoot};

fn round_trip<T: ReadRoot + WriteRoot>(object: &T, name: &str) -> T {
    let bytes = FileWriter::create("accessors.root")
        .add(object)
        .to_bytes()
        .unwrap();
    FileReader::from_bytes(bytes).unwrap().get(name).unwrap()
}

#[test]
fn an_efficiency_keeps_its_options() {
    let mut e = Efficiency::new(2, 0.0, 2.0)
        .named("eff")
        .with_conf_level(0.9)
        .with_statistic_option(StatisticOption::Wilson)
        .with_weight(2.5)
        .with_beta_alpha(1.5)
        .with_beta_beta(0.5);
    e.fill(true, 0.5);
    let back = round_trip(&e, "eff");
    assert_eq!(back.conf_level(), 0.9);
    assert_eq!(back.statistic_option(), StatisticOption::Wilson);
    assert_eq!(back.weight(), 2.5);
    assert_eq!((back.beta_alpha(), back.beta_beta()), (1.5, 0.5));
}

/// The reduction trait and the method are one sum.
#[test]
fn efficiencies_merge_through_the_reduction_trait() {
    let mut a = Efficiency::new(2, 0.0, 2.0);
    a.fill(true, 0.5);
    let mut b = Efficiency::new(2, 0.0, 2.0);
    b.fill(false, 0.5);
    Mergeable::merge(&mut a, &b).unwrap();
    assert_eq!(a.efficiency(1), Some(0.5));
    assert!(Mergeable::merge(&mut a, &Efficiency::new(3, 0.0, 2.0)).is_err());
}

#[test]
fn a_profile_keeps_its_error_mode_and_range() {
    let mut p = Hist::reg(2, 0.0, 2.0).mean().named("p");
    p.fill_sample(0.5, 3.0);
    let p: Profile1D = p.to_root().unwrap();
    let p = p
        .with_error_mode(ErrorMode::Spread)
        .with_ymin(-1.0)
        .with_ymax(9.0);
    let back = round_trip(&p, "p");
    assert_eq!(back.error_mode(), ErrorMode::Spread);
    assert_eq!((back.ymin(), back.ymax()), (-1.0, 9.0));

    let mut p2 = Hist::reg(2, 0.0, 2.0).reg(2, 0.0, 2.0).mean().named("p2");
    p2.fill_sample(0.5, 0.5, 3.0);
    let p2: Profile2D = p2.to_root().unwrap();
    let p2 = p2
        .with_error_mode(ErrorMode::SpreadI)
        .with_zmin(-2.0)
        .with_zmax(8.0);
    let back = round_trip(&p2, "p2");
    assert_eq!(back.error_mode(), ErrorMode::SpreadI);
    assert_eq!((back.zmin(), back.zmax()), (-2.0, 8.0));

    let mut p3 = Hist::reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .reg(2, 0.0, 2.0)
        .mean()
        .named("p3");
    p3.fill_sample(0.5, 0.5, 0.5, 3.0);
    let p3: Profile3D = p3.to_root().unwrap();
    let p3 = p3
        .with_error_mode(ErrorMode::Spread)
        .with_tmin(-3.0)
        .with_tmax(7.0);
    let back = round_trip(&p3, "p3");
    assert_eq!(back.error_mode(), ErrorMode::Spread);
    assert_eq!((back.tmin(), back.tmax()), (-3.0, 7.0));
}

#[test]
fn a_multi_error_graph_keeps_its_summation_mode() {
    let g = MultiErrorGraph::new(
        vec![1.0],
        vec![2.0],
        vec![0.1],
        vec![0.1],
        vec![0.2],
        vec![0.3],
    )
    .unwrap()
    .named("g")
    .with_sum_errors_mode(SumErrorsMode::SquareSum);
    assert_eq!(
        round_trip(&g, "g").sum_errors_mode(),
        SumErrorsMode::SquareSum
    );
}

#[test]
fn an_axis_is_named_titled_and_asked_for_its_edges() {
    let mut axis = Axis::new(4, 0.0, 2.0).named("x").titled("p_T [GeV]");
    assert_eq!((axis.name(), axis.title()), ("x", "p_T [GeV]"));
    axis.set_title("mass");
    assert_eq!(axis.title(), "mass");

    assert_eq!(axis.edge(0), Some(0.0));
    assert_eq!(axis.edge(4), Some(2.0));
    assert_eq!(axis.edge(5), None, "past the last boundary");
    let variable = Axis::variable(&[0.0, 1.0, 5.0]).unwrap();
    assert_eq!((variable.edge(2), variable.edge(3)), (Some(5.0), None));

    assert!(axis.labels().is_empty() && !axis.is_time());
    axis.set_label(2, "second").unwrap();
    assert_eq!(axis.labels()[1], "second");
    axis.set_time_format("%H:%M");
    assert!(axis.is_time());
    assert_eq!(axis.time_format(), "%H:%M");

    // The title set in place is the one a histogram's axis is written with.
    let mut h = Hist::reg(4, 0.0, 2.0)
        .double()
        .named("h")
        .to_root()
        .unwrap();
    h.xaxis_mut().set_title("mass");
    assert_eq!(round_trip(&h, "h").xaxis().title(), "mass");
}
