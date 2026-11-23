//! Efficiency: read a ROOT-written fixture + self-round-trip (ROOT C++ oracle;
//! uproot can't read Efficiency).

use std::path::PathBuf;

use oxiroot_hist::Efficiency;
use oxiroot_io_core::{Compression, FileReader};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn sample() -> Efficiency {
    let mut e = Efficiency::new(4, 0.0, 4.0)
        .named("eff")
        .titled("my eff;x;#epsilon");
    e.fill(true, 0.5);
    e.fill(false, 0.5); // bin1 1/2
    e.fill(true, 1.5);
    e.fill(true, 1.5); // bin2 2/2
    e.fill(false, 2.5); // bin3 0/1
    e.fill(true, 3.5);
    e.fill(true, 3.5);
    e.fill(true, 3.5); // bin4 3/3
    e
}

#[test]
fn reads_root_written_tefficiency() {
    let f = FileReader::open(fixture("tefficiency.root")).expect("open");
    assert_eq!(f.key("eff").unwrap().class_name, "TEfficiency");
    let e = f.get::<Efficiency>("eff").expect("read");
    assert_eq!(e.efficiency(1), Some(0.5));
    assert_eq!(e.efficiency(2), Some(1.0));
    assert_eq!(e.efficiency(3), Some(0.0));
    assert_eq!(e.efficiency(4), Some(1.0));
    assert!((e.conf_level() - 0.682689492137086).abs() < 1e-12);
}

#[test]
fn tefficiency_round_trips() {
    let e = sample();
    assert_eq!(e.efficiency(1), Some(0.5));
    let out = PathBuf::from("/tmp/oxiroot_tefficiency.root");
    oxiroot_io_core::FileWriter::create(&out)
        .add(&e)
        .compression(Compression::Stored)
        .write()
        .expect("write");
    let f = FileReader::open(&out).expect("reopen");
    assert_eq!(f.get::<Efficiency>("eff").unwrap(), e, "round-trips");
}

/// A bin nobody filled has no efficiency, and neither has a bin that does not
/// exist; a bin with trials and no pass has efficiency zero.
#[test]
fn a_bin_without_trials_has_no_efficiency() {
    let mut e = Efficiency::new(2, 0.0, 2.0);
    e.fill(false, 0.5);
    assert_eq!(e.efficiency(1), Some(0.0));
    assert_eq!(e.efficiency(2), None);
    assert_eq!(e.efficiency(99), None);
}

/// ROOT's statistic options map to and from their codes; a code ROOT does not
/// define is an error, not a silent default.
#[test]
fn statistic_options_round_trip_their_codes() {
    use oxiroot_hist::StatisticOption;

    for code in 0..=8 {
        assert_eq!(StatisticOption::from_code(code).unwrap().code(), code);
    }
    assert_eq!(
        StatisticOption::from_code(0).unwrap(),
        StatisticOption::ClopperPearson
    );
    assert_eq!(
        StatisticOption::from_code(8).unwrap(),
        StatisticOption::MidP
    );
    assert!(StatisticOption::from_code(9).is_err());
    assert!(StatisticOption::from_code(-1).is_err());
}

/// Efficiencies ROOT 6.40 filled with weights, and the bounds it gives each
/// bin (`GetEfficiency ∓ GetEfficiencyError{Low,Up}`), from
/// `scripts/gen_efficiency_weighted.C`. Bin 4 holds no trials.
#[test]
fn weighted_and_shortest_intervals_are_those_of_root() {
    let file = FileReader::open(fixture("efficiency_weighted.root")).unwrap();
    type Bounds = [(f64, f64); 3];
    let cases: [(&str, bool, bool, Bounds); 7] = [
        // A weighted Clopper-Pearson is the normal approximation, as in ROOT.
        (
            "cp_weighted",
            true,
            false,
            [
                (0.409786392795, 0.74172875872),
                (0.00063951104699, 0.285074774667),
                (1.0, 1.0),
            ],
        ),
        (
            "normal_weighted",
            true,
            false,
            [
                (0.409786392795, 0.74172875872),
                (0.00063951104699, 0.285074774667),
                (1.0, 1.0),
            ],
        ),
        (
            "jeffrey_weighted",
            true,
            false,
            [
                (0.411562837177, 0.724195885577),
                (0.0486096496099, 0.403653743841),
                (0.808416213809, 0.995728054003),
            ],
        ),
        (
            "uniform_weighted",
            true,
            false,
            [
                (0.41238420286, 0.711130498468),
                (0.0933207409606, 0.452961245298),
                (0.711597977865, 0.968626828123),
            ],
        ),
        (
            "wilson_weight_one",
            true,
            false,
            [
                (0.424835592063, 0.744975728691),
                (0.00953687883754, 0.354099484799),
                (1.0, 1.0),
            ],
        ),
        // A weight set on an unweighted efficiency changes nothing.
        (
            "cp_setweight",
            false,
            false,
            [
                (0.39530770522, 0.780086353313),
                (0.0422249266737, 0.618546137188),
                (0.691840295304, 1.0),
            ],
        ),
        (
            "jeffrey_shortest",
            false,
            true,
            [
                (0.456952442104, 0.751509784233),
                (0.037139073404, 0.402032571842),
                (0.909175480341, 1.0),
            ],
        ),
    ];
    for (name, weighted, shortest, expected) in cases {
        let e = file.get::<Efficiency>(name).unwrap();
        assert_eq!(e.is_weighted(), weighted, "{name}");
        assert_eq!(e.shortest_interval(), shortest, "{name}");
        let tolerance = if shortest { 1e-6 } else { 1e-9 };
        for (bin, (lo, hi)) in (1..).zip(expected) {
            let (l, h) = e.interval(bin).unwrap().unwrap();
            assert!(
                (l - lo).abs() < tolerance && (h - hi).abs() < tolerance,
                "{name} bin {bin}: ({l}, {h}), ROOT ({lo}, {hi})"
            );
        }
        assert_eq!(e.interval(4).unwrap(), None, "{name}");
    }
}

/// The settings survive a write: ROOT reads them from the same bits.
#[test]
fn weighted_and_shortest_settings_round_trip() {
    let file = FileReader::open(fixture("efficiency_weighted.root")).unwrap();
    for name in ["jeffrey_weighted", "jeffrey_shortest", "cp_setweight"] {
        let e = file.get::<Efficiency>(name).unwrap();
        let bytes = oxiroot_io_core::FileWriter::create("e.root")
            .add(&e)
            .compression(Compression::Stored)
            .to_bytes()
            .unwrap();
        let back = FileReader::from_bytes(bytes)
            .unwrap()
            .get::<Efficiency>(name)
            .unwrap();
        assert_eq!(back, e, "{name}");
    }
}
