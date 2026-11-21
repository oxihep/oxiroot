//! `oxroot ls` — list the objects (keys) in a file.

use oxiroot::file::ClassKind;
use std::collections::{HashMap, HashSet};

use clap::Args as ClapArgs;
use oxiroot::FileReader;
use oxiroot::file::raw::Key;
use oxiroot::ntuple::NtupleReader;
use oxiroot::tree::TreeReader;

use crate::json::Json;
use crate::util::{CmdResult, Kind, Report, Table, classify, parse_spec};

/// Arguments for `oxroot ls`.
#[derive(ClapArgs)]
pub struct Args {
    /// The ROOT file (`file.root`).
    file: String,
    /// Long listing: also show each object's cycle and entry count.
    #[arg(short, long)]
    long: bool,
    /// Recurse into every `TDirectory`, showing full `dir/sub/name` paths.
    #[arg(short, long)]
    recursive: bool,
}

/// A guard against runaway recursion on a crafted file (ROOT directory trees are
/// acyclic, so real files never approach this).
const MAX_DEPTH: usize = 64;

/// One listed object.
struct Row {
    /// The name shown: the path, with `;cycle` when the name alone is not
    /// enough to tell two keys apart.
    name: String,
    /// The object's path in the file, `dir/sub/name`.
    path: String,
    class: String,
    title: String,
    cycle: u16,
}

impl Row {
    /// The entry count of a `TTree` or `RNTuple`; `None` for any other class,
    /// or one that cannot be opened.
    fn entries(&self, file: &FileReader) -> Option<u64> {
        match classify(&self.class) {
            Kind::Tree => (TreeReader::open(file, &self.path).ok()).map(|t| t.entries()),
            Kind::RNtuple => (NtupleReader::open(file, &self.path).ok()).map(|n| n.entries()),
            _ => None,
        }
    }
}

/// The keys of a file. Counting entries opens every tree and `RNTuple`, so
/// it waits until a form that shows the counts is asked for.
struct Listing {
    file: FileReader,
    rows: Vec<Row>,
    long: bool,
}

impl Report for Listing {
    fn text(self: Box<Self>) -> String {
        let mut table = if self.long {
            Table::new(&["name", "class", "title", "cycle", "entries"]).right_align(&[3, 4])
        } else {
            Table::new(&["name", "class", "title"])
        };
        for r in &self.rows {
            let mut cells = vec![r.name.clone(), r.class.clone(), r.title.clone()];
            if self.long {
                cells.push(r.cycle.to_string());
                let entries = r.entries(&self.file);
                cells.push(entries.map_or_else(|| "-".to_string(), |e| e.to_string()));
            }
            table.row(cells);
        }
        table.render()
    }

    fn json(self: Box<Self>) -> Json {
        Json::Array(
            (self.rows.iter())
                .map(|r| {
                    let entries = r.entries(&self.file);
                    Json::Object(vec![
                        ("name", Json::s(r.name.clone())),
                        ("class", Json::s(r.class.clone())),
                        ("title", Json::s(r.title.clone())),
                        ("cycle", Json::Int(i64::from(r.cycle))),
                        (
                            "entries",
                            entries.map_or(Json::Null, |e| Json::Int(e.cast_signed())),
                        ),
                    ])
                })
                .collect(),
        )
    }
}

/// Run `oxroot ls`.
pub fn run(args: &Args) -> CmdResult {
    let (path, _) = parse_spec(&args.file);
    let file = crate::util::open_root(&path)?;
    let mut rows = Vec::new();
    collect(&file, "", file.keys(), args.recursive, 0, &mut rows);
    name_ambiguous_cycles(&mut rows);
    Ok(Box::new(Listing {
        file,
        rows,
        long: args.long,
    }))
}

/// Name a key `name;cycle` when the directory holds more than one cycle of it,
/// so the rows can be told apart and pasted straight into `dump` or `show`. A
/// name with one cycle — nearly all of them — is left alone.
fn name_ambiguous_cycles(rows: &mut [Row]) {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for row in rows.iter() {
        *seen.entry(row.name.as_str()).or_default() += 1;
    }
    let ambiguous: HashSet<String> = seen
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|(name, _)| name.to_string())
        .collect();
    for row in rows.iter_mut() {
        if ambiguous.contains(&row.name) {
            row.name = format!("{};{}", row.name, row.cycle);
        }
    }
}

/// Append one [`Row`] per non-deleted key in `dir_path` (`""` for the root
/// directory); with `recursive`, descend into every `TDirectory`, naming keys by
/// their full `dir/sub/name` path.
fn collect(
    file: &FileReader,
    dir_path: &str,
    keys: &[Key],
    recursive: bool,
    depth: usize,
    rows: &mut Vec<Row>,
) {
    for key in keys.iter().filter(|k| !k.is_deleted()) {
        let full = if dir_path.is_empty() {
            key.name.clone()
        } else {
            format!("{dir_path}/{}", key.name)
        };
        rows.push(Row {
            name: full.clone(),
            path: full.clone(),
            class: key.class_name.clone(),
            title: key.title.clone(),
            cycle: key.cycle,
        });
        if recursive
            && depth < MAX_DEPTH
            && ClassKind::of(&key.class_name) == ClassKind::Directory
            && let Ok(sub) = file.subdir(&full)
        {
            collect(file, &full, &sub.keys, recursive, depth + 1, rows);
        }
    }
}
