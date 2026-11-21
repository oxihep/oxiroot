//! `oxroot stat` — a one-screen summary of a ROOT file.

use clap::Args as ClapArgs;

use crate::json::Json;
use crate::util::{Both, CmdResult, compression_label, count, parse_spec, root_version};
use std::fmt::Write as _;

/// Arguments for `oxroot stat`.
#[derive(ClapArgs)]
pub struct Args {
    /// The ROOT file (`file.root`).
    file: String,
}

/// Run `oxroot stat`.
pub fn run(args: &Args) -> CmdResult {
    let (path, _) = parse_spec(&args.file);
    let file = crate::util::open_root(&path)?;
    // The source's own length works for both local files and remote URLs (where
    // `std::fs::metadata` would fail).
    let size = file.size();
    let header = file.header();
    let keys = file.keys().iter().filter(|k| !k.is_deleted()).count();
    let mut streamers: Vec<(String, i32)> = file.streamer_registry().map_or_default(|r| {
        r.infos()
            .iter()
            .map(|i| (i.class_name.clone(), i.class_version))
            .collect()
    });
    streamers.sort();

    let json = {
        let streamers = Json::Array(
            streamers
                .iter()
                .map(|(class, version)| {
                    Json::Object(vec![
                        ("class", Json::s(class.clone())),
                        ("version", Json::Int(i64::from(*version))),
                    ])
                })
                .collect(),
        );
        Json::Object(vec![
            ("file", Json::s(path.clone())),
            ("size_bytes", Json::Int(size.cast_signed())),
            ("root_version", Json::s(root_version(header.version))),
            ("compression", Json::s(compression_label(header.compress))),
            ("objects", count(keys)),
            ("streamers", streamers),
        ])
    };

    // Writing to a `String` cannot fail.
    let mut text = String::new();
    let _ = writeln!(text, "file         {path}");
    let _ = writeln!(text, "size         {}", human_size(size));
    let _ = writeln!(text, "ROOT version {}", root_version(header.version));
    let _ = writeln!(text, "compression  {}", compression_label(header.compress));
    let _ = writeln!(text, "objects      {keys}");
    if !streamers.is_empty() {
        let _ = writeln!(text, "streamers    {} classes", streamers.len());
        for (class, version) in &streamers {
            let _ = writeln!(text, "    {class} (v{version})");
        }
    }
    Both::done(text, json)
}

/// Bytes as a compact human-readable size.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {} ({bytes} B)", UNITS[unit])
    }
}
