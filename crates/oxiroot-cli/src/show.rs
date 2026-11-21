//! `oxroot show` — the structure of a `TTree` or `RNTuple`.

use clap::Args as ClapArgs;
use oxiroot::FileReader;
use oxiroot::ntuple::NtupleReader;
use oxiroot::tree::{BranchLayout, LeafType, TreeReader};

use crate::json::Json;
use crate::util::{Both, CmdResult, Kind, Table, classify, leaf_type_name, parse_spec};

/// Arguments for `oxroot show`.
#[derive(ClapArgs)]
pub struct Args {
    /// The object to show, as `file.root:object`.
    spec: String,
}

/// Run `oxroot show`.
pub fn run(args: &Args) -> CmdResult {
    let (path, obj) = parse_spec(&args.spec);
    let obj = obj.ok_or("show needs an object: `file.root:name`")?;
    let file = crate::util::open_root(&path)?;
    let path = obj.as_str();
    let key = file.key(path)?;
    let (name, class) = (key.name.as_str(), key.class_name.clone());

    match classify(&class) {
        Kind::Tree => show_tree(&file, path, name),
        Kind::RNtuple => show_rntuple(&file, path, name),
        _ => Both::done(
            format!(
                "{name}  {class}\n(not a TTree or RNTuple — use `oxroot dump` to see its contents)\n"
            ),
            Json::Object(vec![("name", Json::s(name)), ("class", Json::s(class))]),
        ),
    }
}

/// Show a `TTree`'s branches, their types, and any unreadable branches.
fn show_tree(file: &FileReader, path: &str, name: &str) -> CmdResult {
    let tree = TreeReader::open(file, path)?;
    let mut branches = Vec::new();
    for b in tree.branch_names() {
        branches.push((b.to_string(), branch_type_label(&tree, b)));
    }
    let unreadable: Vec<(String, String)> = tree
        .unsupported_branches()
        .collect::<Vec<_>>()
        .iter()
        .map(|(n, r)| ((*n).to_string(), (*r).to_string()))
        .collect();

    let json = Json::Object(vec![
        ("name", Json::s(name)),
        ("class", Json::s("TTree")),
        ("entries", Json::Int(tree.entries().cast_signed())),
        ("branches", named_type_array(&branches)),
        (
            "unreadable",
            Json::Array(
                unreadable
                    .iter()
                    .map(|(n, r)| {
                        Json::Object(vec![
                            ("name", Json::s(n.clone())),
                            ("reason", Json::s(r.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);

    let mut text = format!(
        "TTree {name:?}  ({} entries, {} branches)\n",
        tree.entries(),
        branches.len()
    );
    let mut table = Table::new(&["branch", "type"]);
    for (n, t) in &branches {
        table.row(vec![n.clone(), t.clone()]);
    }
    for (n, r) in &unreadable {
        table.row(vec![format!("! {n}"), format!("unreadable ({r})")]);
    }
    text.push_str(&table.render());
    Both::done(text, json)
}

/// Show an `RNTuple`'s top-level fields and their C++ types.
fn show_rntuple(file: &FileReader, path: &str, name: &str) -> CmdResult {
    let ntuple = NtupleReader::open(file, path)?;
    // A top-level field is its own parent. A field nested in another can have
    // the same name, so the name alone does not find the right descriptor.
    let fields: Vec<(String, String)> = ntuple
        .header()
        .top_level_fields()
        .map(|(_, fd)| (fd.name.clone(), fd.type_name.clone()))
        .collect();

    let json = Json::Object(vec![
        ("name", Json::s(name)),
        ("class", Json::s("RNTuple")),
        ("entries", Json::Int(ntuple.entries().cast_signed())),
        ("fields", named_type_array(&fields)),
    ]);

    let mut text = format!(
        "RNTuple {name:?}  ({} entries, {} fields)\n",
        ntuple.entries(),
        fields.len()
    );
    let mut table = Table::new(&["field", "type"]);
    for (n, t) in &fields {
        table.row(vec![n.clone(), t.clone()]);
    }
    text.push_str(&table.render());
    Both::done(text, json)
}

/// A JSON array of `{name, type}` objects.
fn named_type_array(items: &[(String, String)]) -> Json {
    Json::Array(
        items
            .iter()
            .map(|(n, t)| {
                Json::Object(vec![
                    ("name", Json::s(n.clone())),
                    ("type", Json::s(t.clone())),
                ])
            })
            .collect(),
    )
}

/// The C++-ish type label of a branch: `double`, `double[3]` (fixed array),
/// `double[]` (variable), `char*`, or a nested/vector form. Read from the
/// tree's metadata alone: no entry is decoded.
fn branch_type_label(tree: &TreeReader<'_>, name: &str) -> String {
    let leaf = tree.branch_type(name).unwrap_or(LeafType::Str);
    let elem = leaf_type_name(leaf);
    let is_string = leaf == LeafType::Str;

    let shape = tree.branch_shape(name).unwrap_or(&[]);
    if !shape.is_empty() {
        let dims: String = shape.iter().map(|n| format!("[{n}]")).collect();
        return format!("{elem}{dims}");
    }
    match tree.branch_layout(name) {
        Some(BranchLayout::ListOfLists) => format!("std::vector<std::vector<{elem}>>"),
        Some(BranchLayout::List | BranchLayout::CArray) if is_string => {
            "std::vector<string>".to_string()
        }
        Some(BranchLayout::List | BranchLayout::CArray) => format!("{elem}[]"),
        Some(BranchLayout::Array(n)) => format!("{elem}[{n}]"),
        _ if is_string => "char*".to_string(),
        _ => elem.to_string(),
    }
}
