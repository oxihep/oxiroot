//! `oxroot dump` — print an object's data.

use std::fmt::Write as _;

use clap::Args as ClapArgs;
use oxiroot::data::{Column, Dataset};
use oxiroot::file::{ObjString, ParamValue, Parameter};
use oxiroot::hist::{Func1D, Func2D, Func3D, Graph, Hist1D, Hist2D, Hist3D, Profile1D};
use oxiroot::{FileReader, Value};

use crate::json::Json;
use crate::util::{Both, CmdResult, Kind, Table, classify, count, parse_spec};

/// Arguments for `oxroot dump`.
#[derive(ClapArgs)]
pub struct Args {
    /// The object to dump, as `file.root:object`.
    spec: String,
    /// Number of entries / points to print (trees, `RNTuples`, graphs).
    #[arg(short = 'n', long, default_value_t = 10)]
    entries: usize,
    /// Comma-separated branch/field subset to print (default: all).
    #[arg(short, long, value_delimiter = ',')]
    branches: Vec<String>,
}

/// Run `oxroot dump`.
pub fn run(args: &Args) -> CmdResult {
    let (path, obj) = parse_spec(&args.spec);
    let obj = obj.ok_or("dump needs an object: `file.root:name`")?;
    let file = crate::util::open_root(&path)?;
    let path = obj.as_str();
    let key = file.key(path)?;
    let (name, class) = (key.name.as_str(), key.class_name.clone());

    match classify(&class) {
        Kind::Tree => dump_dataset(&file, path, name, "TTree", args),
        Kind::RNtuple => dump_dataset(&file, path, name, "RNTuple", args),
        Kind::Hist1 => dump_th1(&file.get::<Hist1D>(path)?, name),
        Kind::Hist2 => dump_th2(&file.get::<Hist2D>(path)?, name),
        Kind::Hist3 => dump_th3(&file.get::<Hist3D>(path)?, name),
        Kind::Profile => Ok(dump_profile(&file.get::<Profile1D>(path)?, name)),
        Kind::Graph => Ok(dump_graph(&file.get::<Graph>(path)?, name, args.entries)),
        Kind::Function => dump_function(&file, path, name, &class),
        Kind::ObjString => {
            let value = file.get::<ObjString>(path)?.value().to_string();
            Ok(scalar(name, Json::s(value.clone()), &value))
        }
        Kind::Parameter => {
            let (value, text) = param_value(file.get::<Parameter>(path)?.value());
            Ok(scalar(name, value, &text))
        }
        // Any other class: decode it generically from its TStreamerInfo into a
        // dynamic value tree and print it (rootprint-style) — so an unknown class
        // is shown, not refused.
        Kind::Other => {
            let value = file.get::<Value>(path)?;
            Both::done(format!("{value}\n"), value_to_json(&value))
        }
    }
}

/// A report from parts that cannot fail.
fn both(text: String, json: Json) -> Box<dyn crate::util::Report> {
    Box::new(Both { text, json })
}

/// Convert a decoded [`Value`] tree to the CLI's JSON model. Object members
/// become a `members` array of `{name, value}` (dynamic keys can't be `Object`
/// fields, which take `&'static str`).
fn value_to_json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::I8(x) => Json::Int(i64::from(*x)),
        Value::I16(x) => Json::Int(i64::from(*x)),
        Value::I32(x) => Json::Int(i64::from(*x)),
        Value::I64(x) => Json::Int(*x),
        Value::U8(x) => Json::Int(i64::from(*x)),
        Value::U16(x) => Json::Int(i64::from(*x)),
        Value::U32(x) => Json::Int(i64::from(*x)),
        // A u64 above i64::MAX can't be an exact JSON `Int` (which is i64); emit
        // it as a string so the value stays correct rather than wrapping negative.
        Value::U64(x) => u64_json(x),
        Value::F32(x) => Json::F64(f64::from(*x)),
        Value::F64(x) => Json::F64(*x),
        Value::Str(s) => Json::s(s.clone()),
        Value::Array(items) => Json::Array(items.iter().map(value_to_json).collect()),
        Value::Object { class, members } => Json::Object(vec![
            ("class", Json::s(class.clone())),
            (
                "members",
                Json::Array(
                    members
                        .iter()
                        .map(|(n, val)| {
                            Json::Object(vec![
                                ("name", Json::s(n.clone())),
                                ("value", value_to_json(val)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        // A slot pointing at an object written elsewhere in the same object.
        Value::Ref { class } => Json::Object(vec![
            ("class", Json::s(class.clone())),
            ("ref", Json::Bool(true)),
        ]),
        Value::Unsupported { class, reason } => Json::Object(vec![
            ("class", Json::s(class.clone())),
            ("unsupported", Json::s(reason.clone())),
        ]),
        _ => Json::Null,
    }
}

/// A `Func1D`/`Func2D`/`Func3D`: its formula, parameters, and (for `Func1D`) its range.
fn dump_function(file: &FileReader, path: &str, name: &str, class: &str) -> CmdResult {
    // All three share `formula`/`params`; only the read type and range differ.
    let (formula, params, range) = match class {
        "TF2" => {
            let f = file.get::<Func2D>(path)?;
            (f.formula().to_string(), f.params().to_vec(), None)
        }
        "TF3" => {
            let f = file.get::<Func3D>(path)?;
            (f.formula().to_string(), f.params().to_vec(), None)
        }
        _ => {
            let f = file.get::<Func1D>(path)?;
            (
                f.formula().to_string(),
                f.params().to_vec(),
                Some(f.range().into_inner()),
            )
        }
    };

    let ps: Vec<String> = params.iter().map(|p| num(*p)).collect();
    let mut text = format!(
        "{class} {name:?}\nformula  {formula}\nparams   [{}]\n",
        ps.join(", ")
    );
    if let Some((lo, hi)) = range {
        // Writing to a `String` cannot fail.
        let _ = writeln!(text, "range    [{}, {}]", num(lo), num(hi));
    }
    let json = Json::Object(vec![
        ("name", Json::s(name)),
        ("class", Json::s(class)),
        ("formula", Json::s(formula)),
        (
            "params",
            Json::Array(params.iter().map(|p| Json::F64(*p)).collect()),
        ),
        (
            "range",
            range.map_or(Json::Null, |(lo, hi)| {
                Json::Array(vec![Json::F64(lo), Json::F64(hi)])
            }),
        ),
    ]);
    Both::done(text, json)
}

/// A single scalar value: `name = value`, or `{name, value}`.
fn scalar(name: &str, value: Json, text: &str) -> Box<dyn crate::util::Report> {
    both(
        format!("{name} = {text}\n"),
        Json::Object(vec![("name", Json::s(name)), ("value", value)]),
    )
}

/// A `Parameter`'s value as `(json, text)`.
fn param_value(value: ParamValue) -> (Json, String) {
    match value {
        ParamValue::Double(x) => (Json::F64(x), num(x)),
        ParamValue::Float(x) => (Json::F64(f64::from(x)), num(f64::from(x))),
        ParamValue::Int(x) => (Json::Int(i64::from(x)), x.to_string()),
        ParamValue::Long64(x) => (Json::Int(x), x.to_string()),
    }
}

/// The first `n` entries of a `TTree` or an `RNTuple`, read the same way.
fn dump_dataset(
    file: &FileReader,
    path: &str,
    name: &str,
    class: &'static str,
    args: &Args,
) -> CmdResult {
    let dataset = Dataset::open(file, path)?;
    let entries = dataset.entries();
    let n = args
        .entries
        .min(usize::try_from(entries).unwrap_or(usize::MAX));
    let selected: Vec<String> = if args.branches.is_empty() {
        dataset.columns().into_iter().map(str::to_string).collect()
    } else {
        args.branches.clone()
    };
    // One column at a time, and only the clusters covering the first `n`
    // entries. A column asked for by name (`-b`) must read: a mistyped name
    // or a failed read is an error. When every column is dumped, one that
    // cannot be read is left blank so the others still print.
    let mut columns: Vec<(String, Vec<Json>)> = Vec::with_capacity(selected.len());
    for column in selected {
        let read = (dataset
            .batches([&column])
            .range(0..n as u64)
            .step(n as u64)
            .next())
        .transpose()
        .map(|batch| batch.and_then(|mut b| b.remove(&column)));
        let values = match read {
            Ok(Some(values)) => cells(&values),
            Ok(None) => Vec::new(),
            Err(e) if !args.branches.is_empty() => return Err(e.into()),
            Err(_) => Vec::new(),
        };
        columns.push((column, values));
    }

    let mut headers = vec!["#"];
    headers.extend(columns.iter().map(|(c, _)| c.as_str()));
    let mut table = Table::new(&headers).right_align(&[0]);
    for i in 0..n {
        let mut row = vec![i.to_string()];
        row.extend((columns.iter()).map(|(_, v)| v.get(i).map_or_else(String::new, cell_text)));
        table.row(row);
    }
    let text = format!(
        "{class} {name:?}  ({entries} entries; showing {n})\n{}",
        table.render()
    );

    let names = (columns.iter()).map(|(c, _)| Json::s(c.clone())).collect();
    let mut by_column: Vec<_> = (columns.into_iter())
        .map(|(_, values)| {
            values
                .into_iter()
                .chain(std::iter::repeat_with(|| Json::Null))
        })
        .collect();
    let rows = (0..n)
        .map(|_| Json::Array(by_column.iter_mut().filter_map(Iterator::next).collect()))
        .collect();
    let json = Json::Object(vec![
        ("name", Json::s(name)),
        ("class", Json::s(class)),
        ("entries", Json::Int(entries.cast_signed())),
        ("showing", count(n)),
        ("columns", Json::Array(names)),
        ("rows", Json::Array(rows)),
    ]);
    Both::done(text, json)
}

/// A column as one value per entry: the one place a column's shape is taken
/// apart, for the table and the JSON rows alike.
fn cells(column: &Column) -> Vec<Json> {
    fn each<T: Copy>(values: &[T], cell: impl Fn(T) -> Json) -> Vec<Json> {
        values.iter().map(|&x| cell(x)).collect()
    }
    fn int<T: Copy + Into<i64>>(values: &[T]) -> Vec<Json> {
        each(values, |x| Json::Int(x.into()))
    }
    /// `values` cut where `offsets` say, each piece an array.
    fn grouped(offsets: &[u64], values: Vec<Json>) -> Vec<Json> {
        let mut values = values.into_iter();
        (offsets.windows(2))
            .map(|w| {
                let len = usize::try_from(w[1].saturating_sub(w[0])).unwrap_or(usize::MAX);
                Json::Array(values.by_ref().take(len).collect())
            })
            .collect()
    }
    match column {
        Column::Bool(v) => each(v, Json::Bool),
        Column::I8(v) => int(v),
        Column::U8(v) => int(v),
        Column::I16(v) => int(v),
        Column::U16(v) => int(v),
        Column::I32(v) => int(v),
        Column::U32(v) => int(v),
        Column::I64(v) => int(v),
        Column::U64(v) => each(v, |x| u64_json(&x)),
        Column::F32(v) => each(v, |x| Json::F64(f64::from(x))),
        Column::F64(v) => each(v, Json::F64),
        Column::String { offsets, text } => (offsets.windows(2))
            .map(|w| {
                let (a, b) = (
                    usize::try_from(w[0]).unwrap_or(usize::MAX),
                    usize::try_from(w[1]).unwrap_or(usize::MAX),
                );
                Json::s(text.get(a..b).unwrap_or_default())
            })
            .collect(),
        Column::List { offsets, values } => grouped(offsets, cells(values)),
        Column::Array { len, values } => {
            let mut values = cells(values).into_iter();
            let entries = if *len == 0 { 0 } else { values.len() / len };
            (0..entries)
                .map(|_| Json::Array(values.by_ref().take(*len).collect()))
                .collect()
        }
        Column::Optional { valid, values } => {
            let mut values = cells(values).into_iter();
            (valid.iter())
                .map(|&has| has.then(|| values.next()).flatten().unwrap_or(Json::Null))
                .collect()
        }
        Column::Struct(members) => {
            let mut members: Vec<(&String, std::vec::IntoIter<Json>)> = (members.iter())
                .map(|(name, column)| (name, cells(column).into_iter()))
                .collect();
            (0..column.len())
                .map(|_| {
                    Json::Map(
                        (members.iter_mut())
                            .map(|(name, values)| {
                                ((*name).clone(), values.next().unwrap_or(Json::Null))
                            })
                            .collect(),
                    )
                })
                .collect()
        }
        // A column shape added later prints blank until it is given a form.
        _ => Vec::new(),
    }
}

/// A cell of the text table. A string stands as it is; inside a list or a
/// record it is quoted, so its end shows.
fn cell_text(cell: &Json) -> String {
    fn inner(cell: &Json) -> String {
        match cell {
            Json::Str(s) => format!("{s:?}"),
            other => cell_text(other),
        }
    }
    match cell {
        Json::Null => String::new(),
        Json::Bool(b) => b.to_string(),
        Json::Int(n) => n.to_string(),
        Json::F64(x) => num(*x),
        Json::Str(s) => s.clone(),
        Json::Array(items) => {
            let parts: Vec<String> = items.iter().map(inner).collect();
            format!("[{}]", parts.join(", "))
        }
        Json::Object(fields) => {
            let parts: Vec<String> = (fields.iter())
                .map(|(k, v)| format!("{k}: {}", inner(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Json::Map(fields) => {
            let parts: Vec<String> = (fields.iter())
                .map(|(k, v)| format!("{k}: {}", inner(v)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

/// Bins, contents, errors, and summary stats of a `Hist1D`.
fn dump_th1(hist: &Hist1D, name: &str) -> CmdResult {
    let edges = hist.edges();
    // The record holds the bins; the statistics are the histogram's.
    let stats = hist.to_hist()?;
    let errors: Vec<f64> = stats.variances(false).iter().map(|v| v.sqrt()).collect();
    // Each bin as (low, high, content, error).
    let bins: Vec<[f64; 4]> = (hist.values().iter().enumerate())
        .map(|(i, &content)| {
            [
                edges.get(i).copied().unwrap_or(f64::NAN),
                edges.get(i + 1).copied().unwrap_or(f64::NAN),
                content,
                errors.get(i).copied().unwrap_or(0.0),
            ]
        })
        .collect();

    let mut table =
        Table::new(&["bin", "low", "high", "content", "error"]).right_align(&[0, 1, 2, 3, 4]);
    for (i, bin) in bins.iter().enumerate() {
        let mut row = vec![(i + 1).to_string()];
        row.extend(bin.iter().map(|&x| num(x)));
        table.row(row);
    }
    let text = format!(
        "TH1 {name:?}  entries {}  mean {}  std {}  integral {}\n{}",
        num(hist.entries()),
        num(stats.mean()),
        num(stats.std_dev()),
        num(stats.integral(false)),
        table.render()
    );

    let json = Json::Object(vec![
        ("name", Json::s(name)),
        ("class", Json::s("TH1")),
        ("entries", Json::F64(hist.entries())),
        ("mean", Json::F64(stats.mean())),
        ("std", Json::F64(stats.std_dev())),
        ("integral", Json::F64(stats.integral(false))),
        (
            "bins",
            Json::Array(
                (bins.iter())
                    .map(|&[low, high, content, error]| {
                        Json::Object(vec![
                            ("low", Json::F64(low)),
                            ("high", Json::F64(high)),
                            ("content", Json::F64(content)),
                            ("error", Json::F64(error)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    Both::done(text, json)
}

/// A summary of a `Hist2D` (its full grid is not printed).
fn dump_th2(hist: &Hist2D, name: &str) -> CmdResult {
    // The record holds the bins; the statistics are the histogram's.
    let filled = hist.to_hist()?;
    let stats = filled.fill_stats().unwrap_or_else(|| filled.binned_stats());
    let integral: f64 = filled.values(false).iter().sum();
    let (nx, ny) = (hist.nx(), hist.ny());
    let text = format!(
        "TH2 {name:?}  {nx} x {ny} bins\nentries {}  integral {}  mean_x {}  mean_y {}\n",
        num(hist.entries()),
        num(integral),
        num(stats.mean(0)),
        num(stats.mean(1)),
    );
    let json = Json::Object(vec![
        ("name", Json::s(name)),
        ("class", Json::s("TH2")),
        ("bins_x", count(nx)),
        ("bins_y", count(ny)),
        ("entries", Json::F64(hist.entries())),
        ("integral", Json::F64(integral)),
        ("mean_x", Json::F64(stats.mean(0))),
        ("mean_y", Json::F64(stats.mean(1))),
    ]);
    Both::done(text, json)
}

/// A summary of a `Hist3D` (its full grid is not printed).
fn dump_th3(hist: &Hist3D, name: &str) -> CmdResult {
    // The record holds the bins; the statistics are the histogram's.
    let filled = hist.to_hist()?;
    let stats = filled.fill_stats().unwrap_or_else(|| filled.binned_stats());
    let integral: f64 = filled.values(false).iter().sum();
    let (nx, ny, nz) = (hist.nx(), hist.ny(), hist.nz());
    let text = format!(
        "TH3 {name:?}  {nx} x {ny} x {nz} bins\nentries {}  integral {}  mean_x {}  mean_y {}\n",
        num(hist.entries()),
        num(integral),
        num(stats.mean(0)),
        num(stats.mean(1)),
    );
    let json = Json::Object(vec![
        ("name", Json::s(name)),
        ("class", Json::s("TH3")),
        ("bins_x", count(nx)),
        ("bins_y", count(ny)),
        ("bins_z", count(nz)),
        ("entries", Json::F64(hist.entries())),
        ("integral", Json::F64(integral)),
        ("mean_x", Json::F64(stats.mean(0))),
        ("mean_y", Json::F64(stats.mean(1))),
    ]);
    Both::done(text, json)
}

/// Per-bin mean-y of a `Profile1D`.
fn dump_profile(profile: &Profile1D, name: &str) -> Box<dyn crate::util::Report> {
    let edges = profile.edges();
    let low = |i: usize| edges.get(i).copied().unwrap_or(f64::NAN);
    let high = |i: usize| edges.get(i + 1).copied().unwrap_or(f64::NAN);

    let mut table = Table::new(&["bin", "low", "high", "mean-y"]).right_align(&[0, 1, 2, 3]);
    for (i, &value) in profile.values().iter().enumerate() {
        table.row(vec![
            (i + 1).to_string(),
            num(low(i)),
            num(high(i)),
            num(value),
        ]);
    }
    let bins = (profile.values().iter().enumerate())
        .map(|(i, &v)| {
            Json::Object(vec![
                ("low", Json::F64(low(i))),
                ("high", Json::F64(high(i))),
                ("mean_y", Json::F64(v)),
            ])
        })
        .collect();
    both(
        format!("TProfile {name:?}\n{}", table.render()),
        Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TProfile")),
            ("bins", Json::Array(bins)),
        ]),
    )
}

/// The first `n` points of a `Graph`.
fn dump_graph(graph: &Graph, name: &str, n: usize) -> Box<dyn crate::util::Report> {
    // Bound by both coordinate lengths: a graph decoded from a corrupt file can
    // have `fX.len() != fY.len()`, and indexing the shorter one would panic.
    let points = || graph.x().iter().zip(graph.y()).take(n);
    let mut table = Table::new(&["#", "x", "y"]).right_align(&[0, 1, 2]);
    for (i, (&x, &y)) in points().enumerate() {
        table.row(vec![i.to_string(), num(x), num(y)]);
    }
    let json_points = points()
        .map(|(&x, &y)| Json::Object(vec![("x", Json::F64(x)), ("y", Json::F64(y))]))
        .collect();
    both(
        format!(
            "TGraph {name:?}  ({} points)\n{}",
            graph.x().len(),
            table.render()
        ),
        Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TGraph")),
            ("points", Json::Array(json_points)),
        ]),
    )
}

/// A `u64` as JSON: an integer, or a string above `i64::MAX`, which the
/// integer cell cannot hold (so the value is not printed as a negative one).
fn u64_json(x: &u64) -> Json {
    i64::try_from(*x).map_or_else(|_| Json::s(x.to_string()), Json::Int)
}

/// Format a float compactly: integers without a fraction, otherwise up to six
/// decimals with trailing zeros trimmed.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a whole number below 1e15: exact as an i64"
)]
fn num(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return (x as i64).to_string();
    }
    // Six decimals would print a small value as 0 and pad a huge one.
    if x.abs() < 1e-4 || x.abs() >= 1e15 {
        return format!("{x:e}");
    }
    let s = format!("{x:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::num;

    #[test]
    fn num_keeps_values_six_decimals_would_lose() {
        assert_eq!(num(0.0), "0");
        assert_eq!(num(42.0), "42");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(0.000_123), "0.000123");
        assert_eq!(num(2.5e-9), "2.5e-9");
        assert_eq!(num(1.25e20), "1.25e20");
    }
}
