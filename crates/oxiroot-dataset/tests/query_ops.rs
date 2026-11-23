//! Every operation of the query language against the value computed by hand,
//! entry by entry; every check error; and closures of every argument type.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "small test values, compared exactly where exact"
)]

use oxiroot_dataset::query::{Expr, QueryError, col, lit, when};
use oxiroot_dataset::{Batch, Column, Dataset, Format, NamedBatch};
use oxiroot_io_core::{FileReader, FileWriter};

const N: usize = 60;

/// One entry: an integer, a float, a bool, a list, an optional.
struct Row {
    i: i32,
    f: f64,
    b: bool,
    l: Vec<f64>,
    o: Option<f64>,
}

fn rows() -> Vec<Row> {
    (0..N)
        .map(|k| {
            let f = (k as f64 - 30.0) * 0.37;
            Row {
                i: (k % 7) as i32,
                f,
                b: k % 3 == 0,
                l: (0..k % 4).map(|j| f + j as f64 - 1.0).collect(),
                o: (k % 5 != 0).then_some(f * 2.0),
            }
        })
        .collect()
}

fn file(rows: &[Row]) -> FileReader {
    let b = Batch::new()
        .column("i", rows.iter().map(|r| r.i).collect::<Vec<_>>())
        .unwrap()
        .column("f", rows.iter().map(|r| r.f).collect::<Vec<_>>())
        .unwrap()
        .column("b", rows.iter().map(|r| r.b).collect::<Vec<_>>())
        .unwrap()
        .column("l", rows.iter().map(|r| r.l.clone()).collect::<Vec<_>>())
        .unwrap()
        .column("o", rows.iter().map(|r| r.o).collect::<Vec<_>>())
        .unwrap()
        .column(
            "u",
            (0..N as u64).map(|k| k + (1 << 40)).collect::<Vec<_>>(),
        )
        .unwrap()
        .column("s", (0..N).map(|k| format!("s{k}")).collect::<Vec<_>>())
        .unwrap()
        .column("a", (0..N).map(|k| [k as f32, 1.0]).collect::<Vec<_>>())
        .unwrap();
    let bytes = FileWriter::create("o.root")
        .add(NamedBatch::new("t", Format::Ntuple, b))
        .to_bytes()
        .unwrap();
    FileReader::from_bytes(bytes).unwrap()
}

/// A one-value-per-entry output column as numbers, `None` where missing.
fn numbers(c: &Column) -> Vec<Option<f64>> {
    match c {
        Column::F64(v) => v.iter().map(|&x| Some(x)).collect(),
        Column::I64(v) => v.iter().map(|&x| Some(x as f64)).collect(),
        Column::Bool(v) => v.iter().map(|&x| Some(f64::from(u8::from(x)))).collect(),
        Column::Optional { valid, values } => {
            let dense = numbers(values);
            let mut it = dense.into_iter();
            valid
                .iter()
                .map(|&v| if v { it.next().flatten() } else { None })
                .collect()
        }
        other => panic!("not one value per entry: {}", other.type_name()),
    }
}

fn same(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => {
            x == y || (x.is_nan() && y.is_nan()) || (x - y).abs() <= 1e-12 * x.abs().max(1.0)
        }
        (None, None) => true,
        _ => false,
    }
}

type Want = fn(&Row) -> Option<f64>;

fn b2f(b: bool) -> Option<f64> {
    Some(f64::from(u8::from(b)))
}

#[test]
fn every_operation_matches_a_hand_computation() {
    let rows = rows();
    let f = file(&rows);
    let ds = Dataset::open(&f, "t").unwrap();
    let cases: Vec<(&str, Want)> = vec![
        ("sqrt(abs(f))", |r| Some(r.f.abs().sqrt())),
        ("exp(f / 10)", |r| Some((r.f / 10.0).exp())),
        ("log(abs(f) + 1)", |r| Some((r.f.abs() + 1.0).ln())),
        ("log10(abs(f) + 1)", |r| Some((r.f.abs() + 1.0).log10())),
        ("sin(f) + cos(f) + tan(f / 20)", |r| {
            Some(r.f.sin() + r.f.cos() + (r.f / 20.0).tan())
        }),
        ("asin(f / 20) + acos(f / 20) + atan(f)", |r| {
            Some((r.f / 20.0).asin() + (r.f / 20.0).acos() + r.f.atan())
        }),
        ("sinh(f / 10) + cosh(f / 10) + tanh(f)", |r| {
            Some((r.f / 10.0).sinh() + (r.f / 10.0).cosh() + r.f.tanh())
        }),
        ("floor(f) + ceil(f) + round(f)", |r| {
            Some(r.f.floor() + r.f.ceil() + r.f.round())
        }),
        ("atan2(f, i + 1) + hypot(f, i)", |r| {
            Some(r.f.atan2(f64::from(r.i + 1)) + r.f.hypot(f64::from(r.i)))
        }),
        ("min(f, i) + max(f, i)", |r| {
            Some(r.f.min(f64::from(r.i)) + r.f.max(f64::from(r.i)))
        }),
        ("min(i, 3) + max(i, 3) + abs(i - 4)", |r| {
            Some(f64::from(r.i.min(3) + r.i.max(3) + (r.i - 4).abs()))
        }),
        ("-i + -f", |r| Some(-f64::from(r.i) - r.f)),
        ("i + 1", |r| Some(f64::from(r.i + 1))),
        ("i * 2 - 1", |r| Some(f64::from(r.i * 2 - 1))),
        ("i / 2", |r| Some(f64::from(r.i) / 2.0)),
        ("i ^ 2 + f ^ 0.5", |r| {
            Some(f64::from(r.i).powi(2) + r.f.powf(0.5))
        }),
        ("f * 2.5 - 1 + i", |r| {
            Some(r.f * 2.5 - 1.0 + f64::from(r.i))
        }),
        ("f > 1", |r| b2f(r.f > 1.0)),
        ("i >= 2", |r| b2f(r.i >= 2)),
        ("i <= 2", |r| b2f(r.i <= 2)),
        ("i < 2", |r| b2f(r.i < 2)),
        ("i == 3", |r| b2f(r.i == 3)),
        ("i != 3", |r| b2f(r.i != 3)),
        ("f < 0", |r| b2f(r.f < 0.0)),
        ("f <= 0", |r| b2f(r.f <= 0.0)),
        ("f >= 0", |r| b2f(r.f >= 0.0)),
        ("f == f", |_| b2f(true)),
        ("f != 0", |r| b2f(r.f != 0.0)),
        ("b == true", |r| b2f(r.b)),
        ("b != false", |r| b2f(r.b)),
        ("not b", |r| b2f(!r.b)),
        ("b and i > 2", |r| b2f(r.b && r.i > 2)),
        ("b or f < 0", |r| b2f(r.b || r.f < 0.0)),
        ("if b then f else i", |r| {
            Some(if r.b { r.f } else { f64::from(r.i) })
        }),
        ("if b then 1 else 2", |r| Some(if r.b { 1.0 } else { 2.0 })),
        ("any(l > 0)", |r| b2f(r.l.iter().any(|&x| x > 0.0))),
        ("all(l > 0)", |r| b2f(r.l.iter().all(|&x| x > 0.0))),
        ("count(l > 0)", |r| {
            Some(r.l.iter().filter(|&&x| x > 0.0).count() as f64)
        }),
        ("sum(l)", |r| Some(r.l.iter().sum())),
        ("max(l)", |r| r.l.iter().copied().reduce(f64::max)),
        ("min(l)", |r| r.l.iter().copied().reduce(f64::min)),
        ("mean(l)", |r| {
            (!r.l.is_empty()).then(|| r.l.iter().sum::<f64>() / r.l.len() as f64)
        }),
        ("len(l)", |r| Some(r.l.len() as f64)),
        ("l[0]", |r| r.l.first().copied()),
        ("l[-1]", |r| r.l.last().copied()),
        ("l[1] ?? 0", |r| Some(r.l.get(1).copied().unwrap_or(0.0))),
        ("sum(l[l > 0])", |r| {
            Some(r.l.iter().filter(|&&x| x > 0.0).sum())
        }),
        ("sum(l * f)", |r| Some(r.l.iter().map(|x| x * r.f).sum())),
        ("sum(l + i)", |r| {
            Some(r.l.iter().map(|x| x + f64::from(r.i)).sum())
        }),
        ("sum(abs(l))", |r| Some(r.l.iter().map(|x| x.abs()).sum())),
        ("count(l > 0 and b)", |r| {
            Some(r.l.iter().filter(|&&x| x > 0.0 && r.b).count() as f64)
        }),
        ("sum(if l > 0 then l else 0)", |r| {
            Some(r.l.iter().map(|&x| if x > 0.0 { x } else { 0.0 }).sum())
        }),
        ("o + 1", |r| r.o.map(|x| x + 1.0)),
        ("o ?? 5", |r| Some(r.o.unwrap_or(5.0))),
        ("o > 2", |r| r.o.map(|x| f64::from(u8::from(x > 2.0)))),
        ("o > 2 or true", |_| b2f(true)),
        ("o > 2 and false", |_| b2f(false)),
        ("o > 2 and true", |r| {
            r.o.map(|x| f64::from(u8::from(x > 2.0)))
        }),
        ("if o > 2 then 1 else 0", |r| {
            r.o.map(|x| if x > 2.0 { 1.0 } else { 0.0 })
        }),
        ("max(l) ?? -1", |r| {
            Some(r.l.iter().copied().reduce(f64::max).unwrap_or(-1.0))
        }),
        ("u - 1099511627776", |r| Some(r.f / 0.37 + 30.0)),
        ("sum(a)", |r| Some(r.f / 0.37 + 30.0 + 1.0)),
        ("true", |_| b2f(true)),
        ("2.5", |_| Some(2.5)),
    ];
    for (text, want) in &cases {
        let out = ds
            .query()
            .step(17)
            .define("x", *text)
            .read()
            .unwrap_or_else(|e| panic!("{text}: {e}"));
        let got = numbers(out.get("x").unwrap());
        assert_eq!(got.len(), N, "{text}: {} entries", out.len());
        for (k, r) in rows.iter().enumerate() {
            assert!(
                same(got[k], want(r)),
                "{text}, entry {k}: {:?} where {:?}",
                got[k],
                want(r)
            );
        }
    }
}

#[test]
fn per_object_outputs_and_their_missing_values() {
    let rows = rows();
    let f = file(&rows);
    let ds = Dataset::open(&f, "t").unwrap();
    let out = ds
        .query()
        .define("doubled", col("l") * 2)
        .define("shifted", col("l") + col("o"))
        .define("flags", col("l").gt(0.0))
        .define("ints", col("l").gt(0.0).count() + col("l").len())
        .read()
        .unwrap();
    let doubled = out.jagged::<f64>("doubled").unwrap();
    let flags = out.jagged::<bool>("flags").unwrap();
    for (k, r) in rows.iter().enumerate() {
        assert_eq!(
            doubled.row(k),
            r.l.iter().map(|x| x * 2.0).collect::<Vec<_>>()
        );
        assert_eq!(
            flags.row(k),
            r.l.iter().map(|&x| x > 0.0).collect::<Vec<_>>()
        );
    }
    // `l + o`, with `o` missing in some entries: a list of optionals.
    let Column::List { values, .. } = out.get("shifted").unwrap() else {
        panic!("a list");
    };
    assert!(matches!(**values, Column::Optional { .. }));
    assert!(matches!(out.get("ints").unwrap(), Column::I64(_)));
}

#[test]
fn every_check_error() {
    let rows = rows();
    let f = file(&rows);
    let ds = Dataset::open(&f, "t").unwrap();
    let message = |e: &str| -> String {
        let err = ds.query().define("x", e).check().unwrap_err();
        let q: QueryError = err.downcast().unwrap_or_else(|e| panic!("{e}"));
        format!("{q}")
    };
    for (text, expect) in [
        ("-b", "`-` needs a number, not one bool per entry"),
        ("not i", "`not` needs a bool, not one integer per entry"),
        ("i and b", "`and` needs bools, not one integer per entry"),
        ("b == 1", "`==` compares a bool with a integer"),
        ("b < 1", "`<` needs numbers, not one bool per entry"),
        ("b / 2", "`/` needs numbers"),
        ("sqrt(b)", "`sqrt` needs numbers, not one bool per entry"),
        (
            "any(l)",
            "`any` needs a bool per object, not one float per object of `l`",
        ),
        ("sum(l > 0)", "`sum` needs numbers"),
        ("i[0]", "`[i]` picks an object of each entry"),
        ("l[i > 1]", "`x[mask]` keeps objects"),
        ("i ?? true", "`??` mixes a integer and a bool"),
        ("if i then 1 else 2", "`if` needs a bool condition"),
        (
            "if b then 1 else true",
            "`if … then … else` mixes a integer and a bool",
        ),
        ("s + 1", "column `s` holds"),
        ("nope", "no column `nope`"),
        ("l.x", "no column `l.x`"),
    ] {
        let m = message(text);
        assert!(m.starts_with(expect), "{text}: {m}");
    }
    // Text that does not parse.
    for (text, expect) in [
        ("", "the expression is empty"),
        ("`a", "`` ` `` is never closed"),
        ("a = 1", "unexpected `=`"),
        ("a | b", "unexpected `|`"),
        ("~a", "unexpected `~`"),
        ("a $ b", "unexpected `$`"),
        (
            "99999999999999999999",
            "`99999999999999999999` is too large for an integer",
        ),
        ("1e999", "`1e999` is too large for a float"),
        ("1 +", "the expression ends too early"),
        ("(1", "the expression ends where `)` was expected"),
        ("(1 2", "expected `)`, found `2`"),
        ("1 2", "unexpected `2` after a complete expression"),
        (
            "if a then b",
            "the expression ends where `else` was expected",
        ),
        ("sqr(a)", "unknown function `sqr`"),
        ("sqrt(a, b)", "`sqrt` takes 1 argument, not 2"),
        ("atan2(a)", "`atan2` takes 2 arguments, not 1"),
        ("any(a, b)", "`any` takes 1 argument, not 2"),
        ("*", "expected a value, found `*`"),
    ] {
        let err = Expr::parse(text).unwrap_err();
        assert!(err.message().starts_with(expect), "{text:?}: {err}");
        assert_eq!(err.expression(), Some(text));
    }
    assert_eq!(
        Expr::parse("sqr(a)").unwrap_err().hint(),
        Some("did you mean `sqrt`?")
    );
}

#[test]
fn the_builder_writes_what_the_text_says() {
    let e = (col("f").sin() + col("f").cos() * col("f").tan() - col("f").exp() / col("f").ln())
        .min_with(col("f").log10())
        .max_with(col("f").atan2(1.0))
        .hypot(col("f").sqrt().abs().floor().ceil().round());
    let text = e.to_string();
    assert_eq!(Expr::parse(&text).unwrap(), e);
    let e = when(col("b"), col("l").at(-1).unwrap_or(0.0), lit(1)).ge(col("i").pow(2));
    assert_eq!(Expr::parse(&e.to_string()).unwrap(), e);
    let e = !(col("b") | col("i").le(2) & col("i").ne(4) & col("f").lt(1.5) & col("f").eq(0.25));
    assert_eq!(Expr::parse(&e.to_string()).unwrap(), e);
    let e =
        col("l").keep(col("l").gt(1.0)).sum() + col("l").mean() + col("l").min() + col("l").all();
    assert_eq!(e.columns(), ["l"]);
    let e = col("l").asin()
        + col("l").acos()
        + col("l").atan()
        + col("l").sinh()
        + col("l").cosh()
        + col("l").tanh();
    assert_eq!(Expr::parse(&e.to_string()).unwrap(), e);
    let e: Expr = 2.0 + col("f") * 3_i32 - 1_i32 / col("f") + (2_i64 - col("i")) * 4.0 / 2_i32;
    assert_eq!(Expr::parse(&e.to_string()).unwrap(), e);
    assert_eq!((-col("f")).to_string(), "-f");
    assert_eq!(lit(true).to_string(), "true");
    assert_eq!(lit(f64::INFINITY).to_string(), "inf");
    assert_eq!(lit(2_u32).to_string(), "2");
    assert_eq!(lit(2.5_f32).to_string(), "2.5");
    assert_eq!(col("a b").to_string(), "`a b`");
    assert_eq!(col("and").to_string(), "`and`");
}

#[test]
fn closures_of_every_type() {
    let rows = rows();
    let f = file(&rows);
    let ds = Dataset::open(&f, "t").unwrap();
    let out = ds
        .query()
        .step(23)
        .define_with("i8", ["i"], |i: i8| i)
        .define_with("i16", ["i"], |i: i16| i)
        .define_with("u8", ["i"], |i: u8| u32::from(i))
        .define_with("u16", ["i"], |i: u16| i64::from(i))
        .define_with("big", ["u"], |u: u64| u)
        .define_with("f32", ["f"], |f: f32| f)
        .define_with("not", ["b"], |b: bool| !b)
        .define_with("opt", ["o"], |o: Option<f64>| o.map(|x| x + 1.0))
        .define_with("ints", ["l"], |l: Vec<f64>| {
            l.iter().map(|&x| x as i32).collect::<Vec<i32>>()
        })
        .define_with("flags", ["l"], |l: Vec<f64>| {
            l.iter().map(|&x| x > 0.0).collect::<Vec<bool>>()
        })
        .define_with("bools", ["flags"], |b: Vec<bool>| b.len() as u16)
        .define_with("small", ["i"], |i: i64| i)
        .define_with(
            "wide",
            ["i", "f", "b", "o", "l", "small"],
            |i: i64, f: f64, b: bool, o: Option<f32>, l: Vec<f32>, u: u32| {
                i as f64
                    + f
                    + f64::from(u8::from(b))
                    + f64::from(o.unwrap_or(0.0))
                    + l.len() as f64
                    + f64::from(u)
            },
        )
        .read()
        .unwrap();
    let opt = numbers(out.get("opt").unwrap());
    let ints = out.jagged::<i64>("ints").unwrap();
    for (k, r) in rows.iter().enumerate() {
        assert_eq!(opt[k], r.o.map(|x| x + 1.0));
        assert_eq!(
            ints.row(k),
            r.l.iter().map(|&x| x as i64).collect::<Vec<_>>()
        );
    }
    let wide = numbers(out.get("wide").unwrap());
    for (k, r) in rows.iter().enumerate() {
        let want = f64::from(r.i)
            + r.f
            + f64::from(u8::from(r.b))
            + f64::from(r.o.unwrap_or(0.0) as f32)
            + r.l.len() as f64
            + (k % 7) as f64;
        assert!(same(wide[k], Some(want)), "{k}: {:?} {want}", wide[k]);
    }
    // `u` is past u32: an error that says which argument and value.
    let e = ds
        .query()
        .define_with("x", ["u"], |u: u32| u)
        .read()
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "argument 1 of a closure, from column `u`: 1099511627776 does not fit u32"
    );
    assert_eq!(
        numbers(out.get("big").unwrap())[0],
        Some((1u64 << 40) as f64)
    );
    let shown = ds
        .query()
        .select(["i"])
        .define("x", "i + 1")
        .filter("x > 2")
        .define_with("y", ["f"], |f: f64| f)
        .filter_with(["b"], |b: bool| b)
        .to_string();
    assert_eq!(
        shown,
        "select i\ndefine x = i + 1\nfilter x > 2\ndefine y = <closure>(f)\nfilter <closure>(b)\n"
    );
    // A filter closure must return a bool; a selected column must exist.
    let e = ds.query().select(["zz"]).check().unwrap_err();
    assert!(e.to_string().contains("no column `zz` to select"), "{e}");
    let e = ds
        .query()
        .define_with("x", ["nope"], |f: f64| f)
        .check()
        .unwrap_err();
    assert!(e.to_string().contains("no column `nope`"), "{e}");
    let e = ds
        .query()
        .define_with("x", ["l"], |o: Option<f64>| o)
        .check()
        .unwrap_err();
    assert!(e.to_string().contains("take it as a `Vec`"), "{e}");
    let e = ds
        .query()
        .define_with("x", ["o"], |o: f64| o)
        .check()
        .unwrap_err();
    assert!(e.to_string().contains("take it as an `Option`"), "{e}");
    let e = ds
        .query()
        .define_with("x", ["b"], |o: f64| o)
        .check()
        .unwrap_err();
    assert!(e.to_string().contains("one bool per entry"), "{e}");
    let e = ds.query().filter("i").check().unwrap_err();
    assert!(
        e.to_string()
            .contains("a filter needs one bool per entry, but this is one integer per entry"),
        "{e}"
    );
    let e = ds
        .query()
        .define("x", "1")
        .define("x", "2")
        .check()
        .unwrap_err();
    assert!(e.to_string().contains("already has a column `x`"), "{e}");
    let e = ds.query().filter("(").read().unwrap_err();
    assert!(e.to_string().contains("ends too early"), "{e}");
}
