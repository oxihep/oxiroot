//! Every error the facade's API returns converts into `oxiroot::Error`, so `?`
//! works in one function that mixes file IO, statistics, formulas and plotting.

use std::error::Error as _;

use oxiroot::stat::{StatError, pearsonr};

fn correlate(x: &[f64], y: &[f64]) -> oxiroot::Result<f64> {
    let (r, _) = pearsonr(x, y)?;
    Ok(r)
}

#[test]
fn statistics_errors_convert() {
    let err = correlate(&[1.0, 2.0, 3.0], &[1.0, 2.0]).unwrap_err();
    assert!(
        matches!(
            err,
            oxiroot::Error::Stat(StatError::LengthMismatch { left: 3, right: 2 })
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        "statistics: paired samples must have the same length, got 3 and 2"
    );
    assert!(err.source().is_some());
    assert!((correlate(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]).unwrap() - 1.0).abs() < 1e-12);
}

#[cfg(feature = "fit")]
#[test]
fn formula_errors_convert() {
    fn model(formula: &str) -> oxiroot::Result<oxiroot::fit::Model> {
        Ok(oxiroot::fit::Model::from_formula("m", formula)?)
    }
    let err = model("[0]*(x").unwrap_err();
    assert!(matches!(err, oxiroot::Error::Formula(_)), "{err:?}");
    assert!(err.to_string().starts_with("invalid formula: "), "{err}");
    assert!(err.source().is_some());
    assert!(model("[0]*x").is_ok());
}

#[cfg(feature = "plot")]
#[test]
fn plotting_errors_convert() {
    fn save(path: &std::path::Path) -> oxiroot::Result<()> {
        let mut ax = oxiroot::plot::Axes::new();
        ax.plot(&[0.0, 1.0], &[0.0, 1.0])?;
        ax.save(path)?;
        Ok(())
    }
    let dir = std::env::temp_dir();
    // An unknown extension is a plotting error, kept whole.
    let err = save(&dir.join("oxiroot_errors_test.bmp")).unwrap_err();
    assert!(
        matches!(err, oxiroot::Error::Plot(oxiroot::plot::Error::UnknownFormat(ref e)) if e == "bmp"),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("plotting: unknown image format"),
        "{err}"
    );
    // An I/O failure keeps its kind.
    let err = save(&dir.join("oxiroot-no-such-dir/x/y.svg")).unwrap_err();
    assert!(
        matches!(
            err,
            oxiroot::Error::Plot(oxiroot::plot::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound
        ),
        "{err:?}"
    );
    assert!(save(&dir.join("oxiroot_errors_test.svg")).is_ok());

    // A plot length mismatch keeps its structure.
    fn line() -> oxiroot::Result<()> {
        oxiroot::plot::Axes::new().plot(&[0.0, 1.0], &[0.0])?;
        Ok(())
    }
    let err = line().unwrap_err();
    assert!(
        matches!(
            err,
            oxiroot::Error::Plot(oxiroot::plot::Error::LengthMismatch {
                what: "plot ys",
                expected: 2,
                found: 1
            })
        ),
        "{err:?}"
    );
}

/// The prelude's `ThreadedHist` takes what the prelude's `Hist` builder makes
/// (review finding 1B H5: this did not compile).
#[test]
fn prelude_threaded_hist_takes_builder_histograms() {
    use oxiroot::prelude::*;
    let acc = ThreadedHist::new(Hist::reg(10, 0.0, 10.0).int64());
    std::thread::scope(|s| {
        for t in 0..3 {
            let acc = &acc;
            s.spawn(move || (0..100).for_each(|i| acc.fill(f64::from((i + t) % 10))));
        }
    });
    let h: Histogram<oxiroot::hist::oxihist::Int64, 1> = acc.merge().unwrap();
    assert_eq!(h.sum(true), 300);
    let root = h.to_root();
    assert_eq!(root.entries, 300.0);
}
