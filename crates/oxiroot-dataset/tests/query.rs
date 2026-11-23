//! Queries over datasets: every result against the same computation done by
//! hand over the events, for a `TTree` and an `RNTuple`, read in several
//! batches; and the checks that refuse a query before reading.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "the sample's counts and values, small and in range"
)]

use oxiroot_dataset::query::{Expr, QueryError, col, lit, when};
use oxiroot_dataset::{Batch, Column, Dataset, Format, NamedBatch};
use oxiroot_io_core::{Error, FileReader, FileWriter};

/// One event of the sample.
struct Ev {
    run: u32,
    event: u64,
    pt: Vec<f32>,
    eta: Vec<f32>,
    jet_pt: Vec<f32>,
    px: f64,
    py: f64,
    weight: f64,
    met: Option<f64>,
    label: String,
}

/// A deterministic sample: 0 to 4 muons and jets per event, some events
/// with no muons, event numbers past 2⁵³.
fn events() -> Vec<Ev> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..3000)
        .map(|i| {
            let n = (next() * 5.0) as usize;
            let j = (next() * 5.0) as usize;
            Ev {
                run: 300_000 + (i / 1000) as u32,
                event: (1u64 << 60) + i as u64,
                pt: (0..n).map(|_| (5.0 + 60.0 * next()) as f32).collect(),
                eta: (0..n).map(|_| (-3.0 + 6.0 * next()) as f32).collect(),
                jet_pt: (0..j).map(|_| (20.0 + 100.0 * next()) as f32).collect(),
                px: -50.0 + 100.0 * next(),
                py: -50.0 + 100.0 * next(),
                weight: 0.5 + next(),
                met: (next() > 0.3).then(|| 100.0 * next()),
                label: format!("ev{i}"),
            }
        })
        .collect()
}

fn list(rows: impl Iterator<Item = Vec<f32>>) -> Column {
    Column::from(rows.collect::<Vec<_>>())
}

/// The sample as a batch, NanoAOD-style (`nMuon`, `Muon_pt`, …), with the
/// optional `met` where the format stores optionals.
fn batch(evs: &[Ev], optional: bool) -> Batch {
    let mut b = Batch::new()
        .column("run", evs.iter().map(|e| e.run).collect::<Vec<_>>())
        .unwrap()
        .column("event", evs.iter().map(|e| e.event).collect::<Vec<_>>())
        .unwrap()
        .column(
            "nMuon",
            evs.iter().map(|e| e.pt.len() as i32).collect::<Vec<_>>(),
        )
        .unwrap()
        .column("Muon_pt", list(evs.iter().map(|e| e.pt.clone())))
        .unwrap()
        .column("Muon_eta", list(evs.iter().map(|e| e.eta.clone())))
        .unwrap()
        .column("Jet_pt", list(evs.iter().map(|e| e.jet_pt.clone())))
        .unwrap()
        .column("px", evs.iter().map(|e| e.px).collect::<Vec<_>>())
        .unwrap()
        .column("py", evs.iter().map(|e| e.py).collect::<Vec<_>>())
        .unwrap()
        .column("weight", evs.iter().map(|e| e.weight).collect::<Vec<_>>())
        .unwrap()
        .column(
            "label",
            evs.iter().map(|e| e.label.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    if optional {
        b = b
            .column("met", evs.iter().map(|e| e.met).collect::<Vec<_>>())
            .unwrap();
    }
    b
}

fn file(format: Format, evs: &[Ev]) -> FileReader {
    let optional = format == Format::Ntuple;
    let bytes = FileWriter::create("q.root")
        .add(NamedBatch::new("Events", format, batch(evs, optional)))
        .to_bytes()
        .unwrap();
    FileReader::from_bytes(bytes).unwrap()
}

/// Both formats, read in batches of 700 entries.
fn each_format(test: impl Fn(&Dataset<'_>, &[Ev], Format)) {
    let evs = events();
    for format in [Format::Tree, Format::Ntuple] {
        let f = file(format, &evs);
        let ds = Dataset::open(&f, "Events").unwrap();
        test(&ds, &evs, format);
    }
}

fn query_error(e: Error) -> QueryError {
    e.downcast::<QueryError>()
        .unwrap_or_else(|e| panic!("not a query error: {e}"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(1.0)
}

#[test]
fn defines_and_filters_match_a_loop_over_the_events() {
    each_format(|ds, evs, format| {
        let good = |e: &Ev| {
            e.pt.iter()
                .zip(&e.eta)
                .any(|(&p, &h)| p > 20.0 && h.abs() < 2.4)
        };
        let out = ds
            .query()
            .step(700)
            .select(["run", "event", "label"])
            .define("pt", (col("px").pow(2) + col("py").pow(2)).sqrt())
            .filter(col("nMuon").ge(2))
            .filter("any(Muon.pt > 20 and abs(Muon.eta) < 2.4)")
            .define(
                "n_good",
                (col("Muon_pt").gt(20.0) & col("Muon_eta").abs().lt(2.4)).count(),
            )
            .define("lead", col("Muon_pt").max())
            .define("ht", col("Jet_pt").sum())
            .read()
            .unwrap();
        let want: Vec<&Ev> = evs.iter().filter(|e| e.pt.len() >= 2 && good(e)).collect();
        assert_eq!(out.len(), want.len(), "{format:?}");
        assert!(want.len() > 100 && want.len() < evs.len());
        assert_eq!(
            out.names().collect::<Vec<_>>(),
            ["run", "event", "label", "pt", "n_good", "lead", "ht"]
        );
        assert_eq!(
            out.values::<u64>("event").unwrap(),
            want.iter().map(|e| e.event).collect::<Vec<_>>()
        );
        assert_eq!(
            out.strings("label").unwrap().get(0),
            Some(want[0].label.as_str())
        );
        let pt = out.values::<f64>("pt").unwrap();
        let n_good = out.values::<i64>("n_good").unwrap();
        let ht = out.values::<f64>("ht").unwrap();
        for (i, e) in want.iter().enumerate() {
            assert!(close(pt[i], e.px.hypot(e.py)));
            let n =
                e.pt.iter()
                    .zip(&e.eta)
                    .filter(|(p, h)| **p > 20.0 && h.abs() < 2.4)
                    .count();
            assert_eq!(n_good[i], n as i64);
            let sum: f64 = e.jet_pt.iter().map(|&p| f64::from(p)).sum();
            assert!(close(ht[i], sum));
        }
        // `lead` can be missing (no muons), so it is optional; here every
        // event has two.
        let Column::Optional { valid, values } = out.get("lead").unwrap() else {
            panic!("lead is optional");
        };
        assert!(valid.iter().all(|&v| v));
        let Column::F64(lead) = &**values else {
            panic!()
        };
        for (l, e) in lead.iter().zip(&want) {
            assert_eq!(*l, f64::from(e.pt.iter().copied().fold(f32::MIN, f32::max)));
        }
    });
}

#[test]
fn objects_reductions_and_missing_values() {
    each_format(|ds, evs, _| {
        let out = ds
            .query()
            .step(700)
            .define("first_eta", col("Muon_eta").at(0).unwrap_or(-99.0))
            .define("last_pt", col("Muon_pt").at(-1))
            .define("mean_pt", col("Muon_pt").mean())
            .define("n", col("Muon_pt").len())
            .define("good_pt", col("Muon_pt").keep(col("Muon_pt").gt(20.0)))
            .define("scaled", col("Muon_pt") * col("weight"))
            .define("has_mu", when(col("nMuon").gt(0), 1.0, 0.0))
            .read()
            .unwrap();
        assert_eq!(out.len(), evs.len());
        let first = out.values::<f64>("first_eta").unwrap();
        let n = out.values::<i64>("n").unwrap();
        let has = out.values::<f64>("has_mu").unwrap();
        let good = out.jagged::<f64>("good_pt").unwrap();
        let scaled = out.jagged::<f64>("scaled").unwrap();
        let Column::Optional {
            valid: last_valid, ..
        } = out.get("last_pt").unwrap()
        else {
            panic!("last_pt is optional");
        };
        for (i, e) in evs.iter().enumerate() {
            assert_eq!(first[i], e.eta.first().map_or(-99.0, |&h| f64::from(h)));
            assert_eq!(n[i], e.pt.len() as i64);
            assert_eq!(has[i], if e.pt.is_empty() { 0.0 } else { 1.0 });
            assert_eq!(last_valid[i], !e.pt.is_empty());
            let want: Vec<f64> =
                e.pt.iter()
                    .filter(|&&p| p > 20.0)
                    .map(|&p| f64::from(p))
                    .collect();
            assert_eq!(good.row(i), want);
            let want: Vec<f64> = e.pt.iter().map(|&p| f64::from(p) * e.weight).collect();
            assert_eq!(scaled.row(i), want);
        }
    });
}

#[test]
fn a_missing_condition_does_not_keep_an_entry() {
    each_format(|ds, evs, _| {
        // No first muon: `Muon_pt[0] > 10` is missing, so the entry goes;
        // `or true` makes it true again.
        let out = ds.query().filter("Muon_pt[0] > 10").read().unwrap();
        let want = evs
            .iter()
            .filter(|e| e.pt.first().is_some_and(|&p| p > 10.0))
            .count();
        assert_eq!(out.len(), want);
        let all = ds.query().filter("Muon_pt[0] > 10 or true").read().unwrap();
        assert_eq!(all.len(), evs.len());
        let none = ds
            .query()
            .filter("Muon_pt[0] > 10 and false")
            .read()
            .unwrap();
        assert_eq!(none.len(), 0);
    });
}

#[test]
fn integers_stay_exact() {
    each_format(|ds, evs, _| {
        let target = evs[1234].event; // past 2⁵³: a float would round it
        let out = ds
            .query()
            .select(["event"])
            .filter(col("event").eq(lit(target as i64)))
            .read()
            .unwrap();
        assert_eq!(out.values::<u64>("event").unwrap(), [target]);
        let next = ds
            .query()
            .define("next", col("event") + 1)
            .filter(col("event").eq(target as i64))
            .read()
            .unwrap();
        assert_eq!(next.values::<i64>("next").unwrap(), [target as i64 + 1]);
    });
}

#[test]
fn text_and_builder_are_the_same_query() {
    each_format(|ds, _, _| {
        let built =
            col("nMuon").ge(1) & (col("Muon_pt").gt(25.0) & col("Muon_eta").abs().lt(2.1)).any();
        let text = "nMuon >= 1 and any(Muon_pt > 25.0 and abs(Muon_eta) < 2.1)";
        assert_eq!(built.to_string(), text);
        assert_eq!(Expr::parse(text).unwrap(), built);
        let a = ds.query().select(["event"]).filter(built).read().unwrap();
        let b = ds.query().select(["event"]).filter(text).read().unwrap();
        assert_eq!(a, b);
    });
}

#[test]
fn the_text_form_round_trips() {
    for text in [
        "a + b * c - d / e",
        "(a + b) * c",
        "a - (b - c)",
        "-a ^ 2",
        "(-a) ^ 2",
        "a ^ b ^ c",
        "(a ^ b) ^ c",
        "-(-a)",
        "not (a > 1) or b < 2 and c == 3",
        "(a > 1) == (b > 2)",
        "x[0] ?? -99",
        "(x ?? 1) + 2",
        "x[x > 3][-1]",
        "if a > 0 then a else -a",
        "(if a > 0 then 1 else 2) + 1",
        "sum(x[x > 1.5]) / len(x)",
        "max(a, b) + max(x) + atan2(a, b)",
        "`weird name` + 1",
        "Muon.pt * 2.0e-3",
        "true and not false",
        "inf > a",
    ] {
        let e = Expr::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        let printed = e.to_string();
        let again = Expr::parse(&printed).unwrap_or_else(|e| panic!("{text} → {printed}: {e}"));
        assert_eq!(again, e, "{text} → {printed}");
    }
}

#[test]
fn records_and_their_members() {
    // An RNTuple collection of records, `Muon` with members `pt` and `eta`.
    let evs = events();
    let muons = Column::list_of_structs([
        ("pt", list(evs.iter().map(|e| e.pt.clone()))),
        ("eta", list(evs.iter().map(|e| e.eta.clone()))),
    ])
    .unwrap();
    let b = Batch::new().column("Muon", muons).unwrap();
    let bytes = FileWriter::create("r.root")
        .add(NamedBatch::new("Events", Format::Ntuple, b))
        .to_bytes()
        .unwrap();
    let f = FileReader::from_bytes(bytes).unwrap();
    let ds = Dataset::open(&f, "Events").unwrap();
    let out = ds
        .query()
        .define("n_central", "count(abs(Muon.eta) < 1.0)")
        .define("pt_sum", "sum(Muon.pt)")
        .read()
        .unwrap();
    let n = out.values::<i64>("n_central").unwrap();
    for (i, e) in evs.iter().enumerate() {
        assert_eq!(n[i], e.eta.iter().filter(|h| h.abs() < 1.0).count() as i64);
    }
    let err = query_error(ds.query().define("x", "Muon.phi").check().unwrap_err());
    assert_eq!(err.message(), "`Muon` has no member `phi`");
    assert_eq!(err.hint(), Some("its members are pt, eta"));
}

#[test]
fn closures_take_typed_columns() {
    each_format(|ds, evs, format| {
        let mut q = ds
            .query()
            .step(700)
            .define_with("mt", ["px", "py", "weight"], |px: f64, py: f64, w: f64| {
                px.hypot(py) * w
            })
            .define_with("ht", ["Jet_pt"], |pts: Vec<f32>| pts.iter().sum::<f32>())
            .filter_with(["run", "nMuon"], |run: u32, n: i32| run >= 300_001 && n > 0);
        if format == Format::Ntuple {
            q = q.define_with("met_or_0", ["met"], |met: Option<f64>| met.unwrap_or(0.0));
        }
        let out = q.read().unwrap();
        let want: Vec<&Ev> = evs
            .iter()
            .filter(|e| e.run >= 300_001 && !e.pt.is_empty())
            .collect();
        assert_eq!(out.len(), want.len());
        let mt = out.values::<f64>("mt").unwrap();
        let ht = out.values::<f64>("ht").unwrap();
        for (i, e) in want.iter().enumerate() {
            assert!(close(mt[i], e.px.hypot(e.py) * e.weight));
            assert_eq!(ht[i], f64::from(e.jet_pt.iter().sum::<f32>()));
        }
        if format == Format::Ntuple {
            let met = out.values::<f64>("met_or_0").unwrap();
            for (i, e) in want.iter().enumerate() {
                assert_eq!(met[i], e.met.unwrap_or(0.0));
            }
        }
    });
}

#[test]
fn batches_stream_what_read_returns() {
    each_format(|ds, _, _| {
        let q = ds
            .query()
            .step(500)
            .select(["event"])
            .filter("len(Jet_pt) >= 2")
            .define("lead_jet", "max(Jet_pt)");
        let whole = q.read().unwrap();
        let mut joined: Option<Batch> = None;
        let mut count = 0;
        for b in q.batches().unwrap() {
            let b = b.unwrap();
            count += 1;
            match &mut joined {
                Some(j) => j.append(b).unwrap(),
                None => joined = Some(b),
            }
        }
        assert!(count > 1, "{count} batch");
        assert_eq!(joined.unwrap(), whole);
    });
}

#[test]
fn mistakes_are_found_before_reading() {
    each_format(|ds, _, format| {
        let err = |q: oxiroot_dataset::Query<'_, '_>| query_error(q.check().unwrap_err());

        let e = err(ds.query().filter("nMuon >= 2 and Muon_pt > 20"));
        assert_eq!(
            e.message(),
            "a filter needs one bool per entry, but this is one bool per object of `Muon_pt`"
        );
        assert_eq!(e.hint(), Some("say which: any(…) or all(…)"));

        let e = err(ds.query().define("x", "Muon_ptt * 2"));
        assert_eq!(e.message(), "no column `Muon_ptt`");
        assert_eq!(e.hint(), Some("did you mean `Muon_pt`?"));
        assert_eq!(e.span(), Some(0..8));

        let e = err(ds.query().define("x", col("nMuon").gt(1) + 1));
        assert_eq!(e.message(), "`+` needs numbers, not one bool per entry");
        assert_eq!(e.expression(), Some("(nMuon > 1) + 1"));
        assert_eq!(e.span(), Some(0..15));

        let e = err(ds.query().define("x", "sum(nMuon)"));
        assert!(
            e.message()
                .starts_with("`sum` reduces the objects of each entry"),
            "{e}"
        );

        let e = err(ds.query().filter("nMuon > 1 & nMuon < 3"));
        assert_eq!(e.hint(), Some("logical and is `and` (or `&&`)"));

        let e = err(ds.query().filter("0 < nMuon < 3"));
        assert_eq!(e.message(), "comparisons do not chain");

        let e = err(ds.query().define("px", "py"));
        assert_eq!(e.message(), "the query already has a column `px`");

        let e = err(ds.query().define_with("x", ["Muon_pt"], |pt: f32| pt));
        assert!(e.message().contains("take it as a `Vec`"), "{e}");

        let e = err(ds.query().define_with("x", ["px", "py"], |px: f64| px));
        assert!(
            e.message()
                .starts_with("a closure of 1 argument is given 2 columns"),
            "{e}"
        );

        if format == Format::Tree {
            // Nothing above read a byte of data: a query that checks reads.
            assert!(ds.query().filter("nMuon > 1").check().is_ok());
        }
    });
}

#[test]
fn objects_of_different_collections_do_not_combine() {
    each_format(|ds, _, _| {
        let e = query_error(
            ds.query()
                .define("x", "Muon_pt + Jet_pt")
                .read()
                .unwrap_err(),
        );
        assert!(
            e.message()
                .starts_with("these have different numbers of objects"),
            "{e}"
        );
        assert_eq!(e.expression(), Some("Muon_pt + Jet_pt"));
    });
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let deep = format!("{}x{}", "(".repeat(10_000), ")".repeat(10_000));
    assert!(Expr::parse(&deep).is_err());
    let nots = format!("{}x", "not ".repeat(10_000));
    assert!(Expr::parse(&nots).is_err());
}

// ---- Actions: results booked on queries, filled in one pass ---------------

use oxihist::Hist;

#[test]
fn actions_match_a_loop_over_the_events() {
    each_format(|ds, evs, _| {
        let muons = ds.query().step(700).filter("nMuon >= 1");
        let n = muons.count();
        let sum_pt = muons.sum("Muon_pt");
        let mean_px = muons.mean("px");
        let lo = muons.min("Muon_pt");
        let hi = muons.max("Muon_pt");
        let pt = muons.fill(Hist::reg(30, 0.0, 75.0).weight(), ["Muon_pt"]);
        let px = muons.fill_weighted(Hist::reg(20, -50.0, 50.0).weight(), ["px"], "weight");
        let pt_eta = muons.fill(
            Hist::reg(10, 0.0, 75.0).reg(12, -3.0, 3.0).weight(),
            ["Muon_pt", "Muon_eta"],
        );

        let kept: Vec<&Ev> = evs.iter().filter(|e| !e.pt.is_empty()).collect();
        let mut want_pt = Hist::reg(30, 0.0, 75.0).weight();
        let mut want_px = Hist::reg(20, -50.0, 50.0).weight();
        let mut want_pt_eta = Hist::reg(10, 0.0, 75.0).reg(12, -3.0, 3.0).weight();
        for e in &kept {
            for (&p, &h) in e.pt.iter().zip(&e.eta) {
                want_pt.fill(f64::from(p));
                want_pt_eta.fill(f64::from(p), f64::from(h));
            }
            want_px.fill_weight(e.px, e.weight);
        }
        let all_pt: Vec<f64> = kept
            .iter()
            .flat_map(|e| e.pt.iter().map(|&p| f64::from(p)))
            .collect();

        // One `get` runs the pass that fills every action booked.
        assert_eq!(n.get().unwrap(), kept.len() as u64);
        assert!(sum_pt.is_ready() && pt_eta.is_ready());
        assert!(close(sum_pt.get().unwrap(), all_pt.iter().sum()));
        let want_mean = kept.iter().map(|e| e.px).sum::<f64>() / kept.len() as f64;
        assert!(close(mean_px.get().unwrap().unwrap(), want_mean));
        assert_eq!(lo.get().unwrap(), all_pt.iter().copied().reduce(f64::min));
        assert_eq!(hi.get().unwrap(), all_pt.iter().copied().reduce(f64::max));
        assert_eq!(pt.get().unwrap(), want_pt);
        assert_eq!(px.get().unwrap(), want_px);
        assert_eq!(pt_eta.get().unwrap(), want_pt_eta);
    });
}

#[test]
fn branches_share_their_steps_and_one_pass() {
    each_format(|ds, evs, _| {
        let base = ds
            .query()
            .step(700)
            .filter_named("a muon", "nMuon >= 1")
            .define("met2", "px^2 + py^2");
        let signal = base.filter_named("east", "px > 0");
        let control = base.filter("px <= 0").filter_named("north", "py > 0");
        let all = base.count();
        let (sr, cr) = (signal.count(), control.count());
        let sr_met = signal.sum("met2");
        let report = control.report();
        let base_report = base.report();
        // The base is unchanged by the branches built on it.
        assert_eq!(
            base.to_string(),
            "filter nMuon >= 1  # a muon\ndefine met2 = px ^ 2 + py ^ 2\n"
        );

        let muon: Vec<&Ev> = evs.iter().filter(|e| !e.pt.is_empty()).collect();
        let east = muon.iter().filter(|e| e.px > 0.0).count() as u64;
        let west = muon.iter().filter(|e| e.px <= 0.0).count() as u64;
        let north_west = muon.iter().filter(|e| e.px <= 0.0 && e.py > 0.0).count() as u64;
        assert_eq!(all.get().unwrap(), muon.len() as u64);
        assert!(sr.is_ready() && cr.is_ready() && report.is_ready());
        assert_eq!(sr.get().unwrap(), east);
        assert_eq!(cr.get().unwrap(), north_west);
        let want: f64 = muon
            .iter()
            .filter(|e| e.px > 0.0)
            .map(|e| e.px * e.px + e.py * e.py)
            .sum();
        assert!(close(sr_met.get().unwrap(), want));

        let report = report.get().unwrap();
        let names: Vec<&str> = report
            .cuts()
            .iter()
            .map(oxiroot_dataset::query::Cut::name)
            .collect();
        assert_eq!(names, ["a muon", "px <= 0", "north"]);
        let counts: Vec<(u64, u64)> = report
            .cuts()
            .iter()
            .map(|c| (c.all(), c.passed()))
            .collect();
        assert_eq!(
            counts,
            [
                (evs.len() as u64, muon.len() as u64),
                (muon.len() as u64, west),
                (west, north_west)
            ]
        );
        assert_eq!(
            report.cut("north").unwrap().efficiency(),
            Some(north_west as f64 / west as f64)
        );
        let text = report.to_string();
        assert!(
            text.lines().count() == 3 && text.contains("cumulative"),
            "{text}"
        );
        assert_eq!(base_report.get().unwrap().cuts().len(), 1);
    });
}

#[test]
fn a_later_action_takes_a_pass_of_its_own() {
    each_format(|ds, evs, _| {
        let q = ds.query().filter("nMuon == 0");
        let first = q.count();
        q.run().unwrap();
        assert!(first.is_ready());
        let second = q.sum("weight");
        assert!(!second.is_ready());
        let none: Vec<&Ev> = evs.iter().filter(|e| e.pt.is_empty()).collect();
        assert_eq!(first.get().unwrap(), none.len() as u64);
        assert!(close(
            second.get().unwrap(),
            none.iter().map(|e| e.weight).sum()
        ));
        // Nothing booked: a run reads nothing.
        q.run().unwrap();
    });
}

#[test]
fn actions_skip_missing_values_and_keep_what_a_histogram_held() {
    each_format(|ds, evs, format| {
        // `Muon_pt[1]` is missing where there are fewer than two muons.
        let q = ds.query();
        let second = q.sum("Muon_pt[1]");
        let empty = q.filter("nMuon > 100").mean("px");
        let mut start = Hist::reg(4, 0.0, 4.0).weight();
        start.fill(0.5);
        let held = q.fill(start, ["nMuon"]);
        let met = (format == Format::Ntuple).then(|| q.sum("met"));

        let want: f64 = evs
            .iter()
            .filter_map(|e| e.pt.get(1))
            .map(|&p| f64::from(p))
            .sum();
        assert!(close(second.get().unwrap(), want));
        assert_eq!(empty.get().unwrap(), None);
        let held = held.get().unwrap();
        let zeros = evs.iter().filter(|e| e.pt.is_empty()).count() as f64;
        assert_eq!(held.values(false)[0], zeros + 1.0);
        if let Some(met) = met {
            assert!(close(
                met.get().unwrap(),
                evs.iter().filter_map(|e| e.met).sum()
            ));
        }
    });
}

#[test]
fn a_mistake_fails_its_own_action_and_not_the_others() {
    each_format(|ds, _, _| {
        let q = ds.query();
        let good = q.count();
        let typo = q.sum("Muon_ptt");
        let unparsed = q.sum("px +");
        let bad_branch = q.filter("px >").count();
        let not_a_cut = q.filter("px").count();
        let mixed = q.fill(
            Hist::reg(4, 0.0, 4.0).reg(4, 0.0, 4.0).weight(),
            ["Muon_pt", "Jet_pt"],
        );

        assert_eq!(good.get().unwrap(), 3000);
        let e = query_error(typo.get().unwrap_err());
        assert_eq!(e.hint(), Some("did you mean `Muon_pt`?"));
        assert!(query_error(unparsed.get().unwrap_err()).span().is_some());
        let e = query_error(bad_branch.get().unwrap_err());
        assert!(e.span().is_some(), "{e}");
        assert!(
            query_error(not_a_cut.get().unwrap_err())
                .message()
                .contains("one bool per entry")
        );
        // Muons and jets differ in number: caught as the pass fills.
        let e = mixed.get().unwrap_err().to_string();
        assert!(e.contains("different numbers of objects"), "{e}");
    });
}

#[test]
fn a_failed_pass_fails_every_action_it_was_filling() {
    each_format(|ds, _, _| {
        // A shared step that fails (muons and jets differ in number) fails
        // the pass, and every action below and beside it.
        let q = ds.query();
        let below = q.filter("any(Muon_pt > Jet_pt)").count();
        let count = q.count();
        let e = q.run().unwrap_err().to_string();
        assert!(e.contains("different numbers of objects"), "{e}");
        assert!(below.is_ready() && count.is_ready());
        assert!(below.get().is_err());
        let e = count.get().unwrap_err().to_string();
        assert!(e.contains("the pass that fills this result failed"), "{e}");
    });
}

#[cfg(feature = "rayon")]
#[test]
fn a_parallel_pass_fills_the_same_results() {
    each_format(|ds, _, _| {
        let book = |parallel: bool| {
            let q = ds.query().step(100).parallel(parallel).filter("nMuon >= 1");
            let h = q.fill_weighted(Hist::reg(30, 0.0, 75.0).weight(), ["Muon_pt"], "weight");
            let (n, s, r) = (q.count(), q.sum("Muon_pt"), q.report());
            (
                h.get().unwrap(),
                n.get().unwrap(),
                s.get().unwrap(),
                r.get().unwrap(),
            )
        };
        let (h1, n1, s1, r1) = book(false);
        let (h2, n2, s2, r2) = book(true);
        assert_eq!((n1, &r1), (n2, &r2));
        assert!(close(s1, s2));
        for (a, b) in h1.values(true).iter().zip(h2.values(true)) {
            assert!(close(*a, b));
        }
    });
}

#[test]
fn collect_is_read_filled_in_the_shared_pass() {
    each_format(|ds, _, _| {
        for parallel in [false, true] {
            let base = ds.query().step(100).parallel(parallel).filter("nMuon >= 2");
            let pairs = base
                .select(["event", "label"])
                .define("lead", "Muon_pt[0] ?? 0")
                .define("lead_over_ht", "(Muon_pt[0] ?? 0) / (sum(Jet_pt) + 1)");
            let rest = base.filter("px > 0").select(["run"]);
            let (table, other, n) = (pairs.collect(), rest.collect(), base.count());
            let table = table.get().unwrap();
            assert!(n.is_ready() && other.is_ready());
            assert_eq!(table, pairs.read().unwrap(), "parallel: {parallel}");
            assert_eq!(other.get().unwrap(), rest.read().unwrap());
            assert_eq!(table.len() as u64, n.get().unwrap());
        }
    });
}
