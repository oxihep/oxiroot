//! `oxroot dump` — print an object's data.

use clap::Args as ClapArgs;
use oxiroot::file::{ObjString, ParamValue, Parameter};
use oxiroot::hist::{Func1D, Func2D, Func3D, Graph, Hist1D, Hist2D, Hist3D, Profile1D};
use oxiroot::ntuple::NtupleReader;
use oxiroot::ntuple::raw::FieldValues;
use oxiroot::tree::TreeReader;
use oxiroot::tree::raw::BranchValues;
use oxiroot::{FileReader, Value};
use oxiroot_io_core::cast::SaturatingFrom;

use crate::json::Json;
use crate::util::{CmdResult, Kind, Table, classify, parse_spec};

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
pub fn run(args: Args, json: bool) -> CmdResult {
    let (path, obj) = parse_spec(&args.spec);
    let obj = obj.ok_or("dump needs an object: `file.root:name`")?;
    let file = crate::util::open_root(&path)?;
    let path = obj.as_str();
    let key = file.key(path)?;
    let (name, class) = (key.name.as_str(), key.class_name.clone());

    match classify(&class) {
        Kind::Tree => dump_tree(&file, path, name, &args, json),
        Kind::RNtuple => dump_rntuple(&file, path, name, &args, json),
        Kind::Hist1 => dump_th1(&file.get::<Hist1D>(path)?, name, json),
        Kind::Hist2 => dump_th2(&file.get::<Hist2D>(path)?, name, json),
        Kind::Hist3 => dump_th3(&file.get::<Hist3D>(path)?, name, json),
        Kind::Profile => dump_profile(&file.get::<Profile1D>(path)?, name, json),
        Kind::Graph => dump_graph(&file.get::<Graph>(path)?, name, args.entries, json),
        Kind::Function => dump_function(&file, path, name, &class, json),
        Kind::ObjString => {
            let value = file.get::<ObjString>(path)?.value().to_string();
            emit_value(name, Json::s(value.clone()), &value, json);
            Ok(())
        }
        Kind::Parameter => {
            let param = file.get::<Parameter>(path)?;
            let (value, text) = param_value(param.value());
            emit_value(name, value, &text, json);
            Ok(())
        }
        // Any other class: decode it generically from its TStreamerInfo into a
        // dynamic value tree and print it (rootprint-style) — so an unknown class
        // is shown, not refused.
        Kind::Other => dump_generic(&file, path, name, json),
    }
}

/// Dump an arbitrary object via the generic, streamer-info-driven reader.
fn dump_generic(file: &FileReader, path: &str, _name: &str, json: bool) -> CmdResult {
    let value = file.get::<Value>(path)?;
    if json {
        println!("{}", value_to_json(&value).render());
    } else {
        println!("{value}");
    }
    Ok(())
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
fn dump_function(file: &FileReader, path: &str, name: &str, class: &str, json: bool) -> CmdResult {
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

    if json {
        let out = Json::Object(vec![
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
        println!("{}", out.render());
        return Ok(());
    }

    println!("{class} {name:?}");
    println!("formula  {formula}");
    let ps: Vec<String> = params.iter().map(|p| num(*p)).collect();
    println!("params   [{}]", ps.join(", "));
    if let Some((lo, hi)) = range {
        println!("range    [{}, {}]", num(lo), num(hi));
    }
    Ok(())
}

/// Print a single scalar value, as JSON `{name, value}` or `name = value`.
fn emit_value(name: &str, value: Json, text: &str, json: bool) {
    if json {
        let out = Json::Object(vec![("name", Json::s(name)), ("value", value)]);
        println!("{}", out.render());
    } else {
        println!("{name} = {text}");
    }
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

/// The first `n` entries of a `TTree`.
fn dump_tree(file: &FileReader, path: &str, name: &str, args: &Args, json: bool) -> CmdResult {
    let tree = TreeReader::open(file, path)?;
    let n = args.entries.min(usize::saturating_from(tree.entries()));
    let selected = select(&args.branches, tree.branch_names());
    let cols: Vec<(String, Option<BranchValues>)> = selected
        .iter()
        .map(|b| {
            let values = tree.branch(b).range(0..n as u64).values();
            Ok((b.clone(), column_or_blank(values, &args.branches)?))
        })
        .collect::<Result<_, oxiroot::Error>>()?;

    if json {
        let out = Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TTree")),
            ("entries", Json::Int(tree.entries().cast_signed())),
            ("showing", Json::Int(i64::saturating_from(n))),
            (
                "columns",
                Json::Array(cols.iter().map(|(c, _)| Json::s(c.clone())).collect()),
            ),
            ("rows", rows_json(&cols, n, branch_json)),
        ]);
        println!("{}", out.render());
        return Ok(());
    }

    println!("TTree {name:?}  ({} entries; showing {n})", tree.entries());
    print_table(&cols, n, branch_cell);
    Ok(())
}

/// The first `n` entries of an `RNTuple`.
fn dump_rntuple(file: &FileReader, path: &str, name: &str, args: &Args, json: bool) -> CmdResult {
    let ntuple = NtupleReader::open(file, path)?;
    let n = args.entries.min(usize::saturating_from(ntuple.entries()));
    let selected = select(&args.branches, ntuple.field_names());
    // Read only the clusters covering the first `n` entries, not the whole field.
    let cols: Vec<(String, Option<FieldValues>)> = selected
        .iter()
        .map(|f| {
            let values = ntuple.field(f).range(0..n as u64).values();
            Ok((f.clone(), column_or_blank(values, &args.branches)?))
        })
        .collect::<Result<_, oxiroot::Error>>()?;

    if json {
        let out = Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("RNTuple")),
            ("entries", Json::Int(ntuple.entries().cast_signed())),
            ("showing", Json::Int(i64::saturating_from(n))),
            (
                "columns",
                Json::Array(cols.iter().map(|(c, _)| Json::s(c.clone())).collect()),
            ),
            ("rows", rows_json(&cols, n, field_json)),
        ]);
        println!("{}", out.render());
        return Ok(());
    }

    println!(
        "RNTuple {name:?}  ({} entries; showing {n})",
        ntuple.entries()
    );
    print_table(&cols, n, field_cell);
    Ok(())
}

/// One column's values, or the reason it could not be read. A column the user
/// asked for by name (`-b`) must read: a mistyped name or a failed read is an
/// error. When every column is dumped, one this crate cannot read is left
/// blank so the others still print.
fn column_or_blank<T>(
    values: oxiroot::file::Result<T>,
    requested: &[String],
) -> Result<Option<T>, oxiroot::Error> {
    match values {
        Ok(v) => Ok(Some(v)),
        Err(e) if !requested.is_empty() => Err(e.into()),
        Err(_) => Ok(None),
    }
}

/// The selected column names (the `-b` subset, or all).
fn select(requested: &[String], all: Vec<&str>) -> Vec<String> {
    if requested.is_empty() {
        all.iter().map(|s| (*s).to_string()).collect()
    } else {
        requested.to_vec()
    }
}

/// Print a `#`-indexed table of `n` rows over `cols`, one cell per column.
fn print_table<T>(cols: &[(String, Option<T>)], n: usize, cell: fn(&T, usize) -> String) {
    let mut headers = vec!["#".to_string()];
    headers.extend(cols.iter().map(|(c, _)| c.clone()));
    let head_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    let mut table = Table::new(&head_refs).right_align(&[0]);
    for i in 0..n {
        let mut row = vec![i.to_string()];
        for (_, values) in cols {
            row.push(values.as_ref().map_or_else(String::new, |v| cell(v, i)));
        }
        table.row(row);
    }
    table.print();
}

/// The JSON `rows` array: `n` arrays of typed cells over `cols`.
fn rows_json<T>(cols: &[(String, Option<T>)], n: usize, cell: fn(&T, usize) -> Json) -> Json {
    Json::Array(
        (0..n)
            .map(|i| {
                Json::Array(
                    cols.iter()
                        .map(|(_, v)| v.as_ref().map_or(Json::Null, |vv| cell(vv, i)))
                        .collect(),
                )
            })
            .collect(),
    )
}

/// Bins, contents, errors, and summary stats of a `Hist1D`.
fn dump_th1(hist: &Hist1D, name: &str, json: bool) -> CmdResult {
    let edges = hist.edges();
    // The record holds the bins; the statistics are the histogram's.
    let stats = hist.to_hist()?;
    let errors: Vec<f64> = stats.variances(false).iter().map(|v| v.sqrt()).collect();
    let bin = |i: usize, content: f64| {
        (
            edges.get(i).copied().unwrap_or(f64::NAN),
            edges.get(i + 1).copied().unwrap_or(f64::NAN),
            content,
            errors.get(i).copied().unwrap_or(0.0),
        )
    };

    if json {
        let bins = Json::Array(
            hist.values()
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    let (low, high, content, error) = bin(i, c);
                    Json::Object(vec![
                        ("low", Json::F64(low)),
                        ("high", Json::F64(high)),
                        ("content", Json::F64(content)),
                        ("error", Json::F64(error)),
                    ])
                })
                .collect(),
        );
        let out = Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TH1")),
            ("entries", Json::F64(hist.entries())),
            ("mean", Json::F64(stats.mean())),
            ("std", Json::F64(stats.std_dev())),
            ("integral", Json::F64(stats.integral(false))),
            ("bins", bins),
        ]);
        println!("{}", out.render());
        return Ok(());
    }

    println!(
        "TH1 {name:?}  entries {}  mean {}  std {}  integral {}",
        num(hist.entries()),
        num(stats.mean()),
        num(stats.std_dev()),
        num(stats.integral(false)),
    );
    let mut table =
        Table::new(&["bin", "low", "high", "content", "error"]).right_align(&[0, 1, 2, 3, 4]);
    for (i, &content) in hist.values().iter().enumerate() {
        let (low, high, content, error) = bin(i, content);
        table.row(vec![
            (i + 1).to_string(),
            num(low),
            num(high),
            num(content),
            num(error),
        ]);
    }
    table.print();
    Ok(())
}

/// A summary of a `Hist2D` (its full grid is not printed).
fn dump_th2(hist: &Hist2D, name: &str, json: bool) -> CmdResult {
    // The record holds the bins; the statistics are the histogram's.
    let filled = hist.to_hist()?;
    let stats = filled.fill_stats().unwrap_or_else(|| filled.binned_stats());
    let integral: f64 = filled.values(false).iter().sum();
    let (nx, ny) = (hist.nx(), hist.ny());
    if json {
        let out = Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TH2")),
            ("bins_x", Json::Int(i64::saturating_from(nx))),
            ("bins_y", Json::Int(i64::saturating_from(ny))),
            ("entries", Json::F64(hist.entries())),
            ("integral", Json::F64(integral)),
            ("mean_x", Json::F64(stats.mean(0))),
            ("mean_y", Json::F64(stats.mean(1))),
        ]);
        println!("{}", out.render());
        return Ok(());
    }
    println!("TH2 {name:?}  {nx} x {ny} bins");
    println!(
        "entries {}  integral {}  mean_x {}  mean_y {}",
        num(hist.entries()),
        num(integral),
        num(stats.mean(0)),
        num(stats.mean(1)),
    );
    Ok(())
}

/// A summary of a `Hist3D` (its full grid is not printed).
fn dump_th3(hist: &Hist3D, name: &str, json: bool) -> CmdResult {
    // The record holds the bins; the statistics are the histogram's.
    let filled = hist.to_hist()?;
    let stats = filled.fill_stats().unwrap_or_else(|| filled.binned_stats());
    let integral: f64 = filled.values(false).iter().sum();
    let (nx, ny, nz) = (hist.nx(), hist.ny(), hist.nz());
    if json {
        let out = Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TH3")),
            ("bins_x", Json::Int(i64::saturating_from(nx))),
            ("bins_y", Json::Int(i64::saturating_from(ny))),
            ("bins_z", Json::Int(i64::saturating_from(nz))),
            ("entries", Json::F64(hist.entries())),
            ("integral", Json::F64(integral)),
            ("mean_x", Json::F64(stats.mean(0))),
            ("mean_y", Json::F64(stats.mean(1))),
        ]);
        println!("{}", out.render());
        return Ok(());
    }
    println!("TH3 {name:?}  {nx} x {ny} x {nz} bins");
    println!(
        "entries {}  integral {}  mean_x {}  mean_y {}",
        num(hist.entries()),
        num(integral),
        num(stats.mean(0)),
        num(stats.mean(1)),
    );
    Ok(())
}

/// Per-bin mean-y of a `Profile1D`.
fn dump_profile(profile: &Profile1D, name: &str, json: bool) -> CmdResult {
    let edges = profile.edges();
    let low = |i: usize| edges.get(i).copied().unwrap_or(f64::NAN);
    let high = |i: usize| edges.get(i + 1).copied().unwrap_or(f64::NAN);

    if json {
        let bins = Json::Array(
            profile
                .values()
                .iter()
                .enumerate()
                .map(|(i, &v)| {
                    Json::Object(vec![
                        ("low", Json::F64(low(i))),
                        ("high", Json::F64(high(i))),
                        ("mean_y", Json::F64(v)),
                    ])
                })
                .collect(),
        );
        let out = Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TProfile")),
            ("bins", bins),
        ]);
        println!("{}", out.render());
        return Ok(());
    }

    println!("TProfile {name:?}");
    let mut table = Table::new(&["bin", "low", "high", "mean-y"]).right_align(&[0, 1, 2, 3]);
    for (i, &value) in profile.values().iter().enumerate() {
        table.row(vec![
            (i + 1).to_string(),
            num(low(i)),
            num(high(i)),
            num(value),
        ]);
    }
    table.print();
    Ok(())
}

/// The first `n` points of a `Graph`.
fn dump_graph(graph: &Graph, name: &str, n: usize, json: bool) -> CmdResult {
    // Bound by both coordinate lengths: a graph decoded from a corrupt file can
    // have `fX.len() != fY.len()`, and indexing the shorter one would panic.
    let count = graph.x().len().min(graph.y().len()).min(n);
    if json {
        let points = Json::Array(
            (0..count)
                .map(|i| {
                    Json::Object(vec![
                        ("x", Json::F64(graph.x()[i])),
                        ("y", Json::F64(graph.y()[i])),
                    ])
                })
                .collect(),
        );
        let out = Json::Object(vec![
            ("name", Json::s(name)),
            ("class", Json::s("TGraph")),
            ("points", points),
        ]);
        println!("{}", out.render());
        return Ok(());
    }

    println!("TGraph {name:?}  ({} points)", graph.x().len());
    let mut table = Table::new(&["#", "x", "y"]).right_align(&[0, 1, 2]);
    for i in 0..count {
        table.row(vec![i.to_string(), num(graph.x()[i]), num(graph.y()[i])]);
    }
    table.print();
    Ok(())
}

/// One `BranchValues` entry as a cell string.
fn branch_cell(values: &BranchValues, i: usize) -> String {
    use BranchValues::*;
    macro_rules! scalar {
        ($v:expr) => {
            $v.get(i).map(ToString::to_string).unwrap_or_default()
        };
    }
    match values {
        Bool(v) => scalar!(v),
        I8(v) => scalar!(v),
        U8(v) => scalar!(v),
        I16(v) => scalar!(v),
        U16(v) => scalar!(v),
        I32(v) => scalar!(v),
        U32(v) => scalar!(v),
        I64(v) => scalar!(v),
        U64(v) => scalar!(v),
        F32(v) => v.get(i).map_or_else(String::new, |x| num(f64::from(*x))),
        F64(v) => v.get(i).map_or_else(String::new, |x| num(*x)),
        Str(v) => v.get(i).cloned().unwrap_or_default(),
        VecBool(v) => list_cell(v, i),
        VecI8(v) => list_cell(v, i),
        VecU8(v) => list_cell(v, i),
        VecI16(v) => list_cell(v, i),
        VecU16(v) => list_cell(v, i),
        VecI32(v) => list_cell(v, i),
        VecU32(v) => list_cell(v, i),
        VecI64(v) => list_cell(v, i),
        VecU64(v) => list_cell(v, i),
        VecF32(v) => list_cell(v, i),
        VecF64(v) => list_cell(v, i),
        VecStr(v) => v.get(i).map_or_else(String::new, |row| format!("{row:?}")),
        Nested { .. } => "...".to_string(),
        _ => "...".to_string(),
    }
}

/// One `FieldValues` entry as a cell string.
fn field_cell(values: &FieldValues, i: usize) -> String {
    use FieldValues::*;
    macro_rules! scalar {
        ($v:expr) => {
            $v.get(i).map(ToString::to_string).unwrap_or_default()
        };
    }
    match values {
        Bool(v) => scalar!(v),
        I8(v) => scalar!(v),
        U8(v) => scalar!(v),
        I16(v) => scalar!(v),
        U16(v) => scalar!(v),
        I32(v) => scalar!(v),
        U32(v) => scalar!(v),
        I64(v) => scalar!(v),
        U64(v) => scalar!(v),
        F32(v) => v.get(i).map_or_else(String::new, |x| num(f64::from(*x))),
        F64(v) => v.get(i).map_or_else(String::new, |x| num(*x)),
        Str(v) => v.get(i).cloned().unwrap_or_default(),
        VecBool(v) => list_cell(v, i),
        VecI8(v) => list_cell(v, i),
        VecU8(v) => list_cell(v, i),
        VecI16(v) => list_cell(v, i),
        VecU16(v) => list_cell(v, i),
        VecI32(v) => list_cell(v, i),
        VecU32(v) => list_cell(v, i),
        VecI64(v) => list_cell(v, i),
        VecU64(v) => list_cell(v, i),
        VecF32(v) => list_cell(v, i),
        VecF64(v) => list_cell(v, i),
        VecStr(v) => v.get(i).map_or_else(String::new, |row| format!("{row:?}")),
        _ => "...".to_string(),
    }
}

/// One `BranchValues` entry as a typed JSON value.
fn branch_json(values: &BranchValues, i: usize) -> Json {
    use BranchValues::*;
    match values {
        Bool(v) => opt(v.get(i), |x| Json::Bool(*x)),
        I8(v) => opt(v.get(i), int),
        U8(v) => opt(v.get(i), int),
        I16(v) => opt(v.get(i), int),
        U16(v) => opt(v.get(i), int),
        I32(v) => opt(v.get(i), int),
        U32(v) => opt(v.get(i), int),
        I64(v) => opt(v.get(i), int),
        U64(v) => opt(v.get(i), u64_json),
        F32(v) => opt(v.get(i), float),
        F64(v) => opt(v.get(i), float),
        Str(v) => opt(v.get(i), |s| Json::s(s.clone())),
        VecBool(v) => arr(v.get(i), |x| Json::Bool(*x)),
        VecI8(v) => arr(v.get(i), int),
        VecU8(v) => arr(v.get(i), int),
        VecI16(v) => arr(v.get(i), int),
        VecU16(v) => arr(v.get(i), int),
        VecI32(v) => arr(v.get(i), int),
        VecU32(v) => arr(v.get(i), int),
        VecI64(v) => arr(v.get(i), int),
        VecU64(v) => arr(v.get(i), u64_json),
        VecF32(v) => arr(v.get(i), float),
        VecF64(v) => arr(v.get(i), float),
        VecStr(v) => arr(v.get(i), |s| Json::s(s.clone())),
        Nested { .. } => Json::Null,
        _ => Json::Null,
    }
}

/// One `FieldValues` entry as a typed JSON value.
fn field_json(values: &FieldValues, i: usize) -> Json {
    use FieldValues::*;
    match values {
        Bool(v) => opt(v.get(i), |x| Json::Bool(*x)),
        I8(v) => opt(v.get(i), int),
        U8(v) => opt(v.get(i), int),
        I16(v) => opt(v.get(i), int),
        U16(v) => opt(v.get(i), int),
        I32(v) => opt(v.get(i), int),
        U32(v) => opt(v.get(i), int),
        I64(v) => opt(v.get(i), int),
        U64(v) => opt(v.get(i), u64_json),
        F32(v) => opt(v.get(i), float),
        F64(v) => opt(v.get(i), float),
        Str(v) => opt(v.get(i), |s| Json::s(s.clone())),
        VecBool(v) => arr(v.get(i), |x| Json::Bool(*x)),
        VecI8(v) => arr(v.get(i), int),
        VecU8(v) => arr(v.get(i), int),
        VecI16(v) => arr(v.get(i), int),
        VecU16(v) => arr(v.get(i), int),
        VecI32(v) => arr(v.get(i), int),
        VecU32(v) => arr(v.get(i), int),
        VecI64(v) => arr(v.get(i), int),
        VecU64(v) => arr(v.get(i), u64_json),
        VecF32(v) => arr(v.get(i), float),
        VecF64(v) => arr(v.get(i), float),
        VecStr(v) => arr(v.get(i), |s| Json::s(s.clone())),
        _ => Json::Null,
    }
}

/// A `u64` as JSON: an integer, or a string above `i64::MAX`, which the
/// integer cell cannot hold (so the value is not printed as a negative one).
fn u64_json(x: &u64) -> Json {
    i64::try_from(*x).map_or_else(|_| Json::s(x.to_string()), Json::Int)
}

/// A JSON integer from any small integer type.
fn int<T: Copy + Into<i64>>(x: &T) -> Json {
    Json::Int((*x).into())
}

/// A JSON float from an `f32`/`f64`.
fn float<T: Copy + Into<f64>>(x: &T) -> Json {
    Json::F64((*x).into())
}

/// `f(x)` if present, else `null`.
fn opt<T, F: Fn(&T) -> Json>(x: Option<&T>, f: F) -> Json {
    x.map_or(Json::Null, f)
}

/// A JSON array of `f` over the per-entry list at `row`, or `null` if absent.
fn arr<T, F: Fn(&T) -> Json>(row: Option<&Vec<T>>, f: F) -> Json {
    row.map_or(Json::Null, |r| Json::Array(r.iter().map(&f).collect()))
}

/// Entry `i` of a per-entry list column as `[a, b, c]`.
fn list_cell<T: ToString>(rows: &[Vec<T>], i: usize) -> String {
    rows.get(i).map_or_else(String::new, |row| {
        let parts: Vec<String> = row.iter().map(ToString::to_string).collect();
        format!("[{}]", parts.join(", "))
    })
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
