//! Shared helpers: path-spec parsing, class classification, type-name mapping,
//! and a minimal aligned-column table.

use oxiroot::file::ClassKind;
use std::error::Error;
use std::path::Path;

use oxiroot::FileReader;
use oxiroot::tree::LeafType;

use crate::json::Json;

/// What a command found, to print as text or as JSON. `main` picks one and
/// prints it; a command never prints.
pub trait Report {
    /// The table or lines a person reads, each line ended.
    fn text(self: Box<Self>) -> String;
    /// The same facts for a program.
    fn json(self: Box<Self>) -> Json;
}

/// A report with both forms already built, for a command whose two forms
/// cost about the same.
pub struct Both {
    pub text: String,
    pub json: Json,
}

impl Both {
    /// The report as a command's result.
    pub fn done(text: String, json: Json) -> CmdResult {
        Ok(Box::new(Self { text, json }))
    }
}

impl Report for Both {
    fn text(self: Box<Self>) -> String {
        self.text
    }
    fn json(self: Box<Self>) -> Json {
        self.json
    }
}

/// Every command returns this: its report, else a boxed error printed by
/// `main`.
pub type CmdResult = Result<Box<dyn Report>, Box<dyn Error>>;

/// A count as a JSON integer.
pub fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

/// Whether `target` is a remote URL (`http(s)://` or `root://`) rather than a
/// local path.
pub fn is_url(target: &str) -> bool {
    ["http://", "https://", "root://", "roots://"]
        .iter()
        .any(|s| target.starts_with(s))
}

/// Open a ROOT file from a local path or a remote URL — `http(s)://` (the `http`
/// feature) or `root://` `XRootD` (the `xrootd` feature) — read lazily via
/// byte-range requests.
pub fn open_root(target: &str) -> Result<FileReader, Box<dyn Error>> {
    if is_url(target) {
        #[cfg(any(feature = "http", feature = "xrootd"))]
        {
            return Ok(FileReader::open_url(target)?);
        }
        #[cfg(not(any(feature = "http", feature = "xrootd")))]
        {
            return Err(format!(
                "{target}: reading from a URL needs the `http` or `xrootd` feature \
                 (rebuild oxroot with `--features http` and/or `--features xrootd`)"
            )
            .into());
        }
    }
    Ok(FileReader::open(target)?)
}

/// Split a `file.root:dir/object` spec into the file path and an optional
/// in-file object path.
///
/// The object path is whatever follows the last `:` — unless that would name a
/// file which doesn't exist while the whole spec does (a `:` inside the
/// filename), in which case the whole spec is treated as the file. For a URL,
/// only a `:` in the final path segment (after the last `/`) separates the
/// object, so the scheme's `://` and any `host:port` colon are not mistaken for
/// it.
pub fn parse_spec(spec: &str) -> (String, Option<String>) {
    if is_url(spec) {
        let seg = spec.rfind('/').map_or(0, |i| i + 1);
        if let Some(rel) = spec[seg..].find(':') {
            let cut = seg + rel;
            let obj = &spec[cut + 1..];
            if !obj.is_empty() {
                return (spec[..cut].to_string(), Some(obj.to_string()));
            }
        }
        return (spec.to_string(), None);
    }
    if let Some((file, obj)) = spec.rsplit_once(':') {
        let split_names_a_file = Path::new(file).exists() || !Path::new(spec).exists();
        if !file.is_empty() && !obj.is_empty() && split_names_a_file {
            return (file.to_string(), Some(obj.to_string()));
        }
    }
    (spec.to_string(), None)
}

/// A coarse object category derived from a ROOT class name — the CLI dispatches
/// `show`/`dump` on this.
pub enum Kind {
    Tree,
    RNtuple,
    Hist1,
    Hist2,
    Hist3,
    Profile,
    Graph,
    Function,
    ObjString,
    Parameter,
    Other,
}

/// Classify a ROOT class name into a [`Kind`].
pub fn classify(class: &str) -> Kind {
    // What the storage layer tells apart comes from its one table; the
    // rest are the object classes this tool prints in their own way.
    match (ClassKind::of(class), class) {
        (ClassKind::Tree, _) => Kind::Tree,
        (ClassKind::Ntuple, _) => Kind::RNtuple,
        (ClassKind::Hist(1), _) => Kind::Hist1,
        (ClassKind::Hist(2), _) => Kind::Hist2,
        (ClassKind::Hist(3), _) => Kind::Hist3,
        (_, "TProfile") => Kind::Profile,
        (_, "TObjString") => Kind::ObjString,
        (_, "TGraph" | "TGraphErrors" | "TGraphAsymmErrors") => Kind::Graph,
        (_, "TF1" | "TF2" | "TF3") => Kind::Function,
        (_, c) if c.starts_with("TParameter") => Kind::Parameter,
        _ => Kind::Other,
    }
}

/// The C++ element type name for a leaf type.
pub fn leaf_type_name(t: LeafType) -> &'static str {
    match t {
        LeafType::Bool => "bool",
        LeafType::I8 => "int8_t",
        LeafType::U8 => "uint8_t",
        LeafType::I16 => "int16_t",
        LeafType::U16 => "uint16_t",
        LeafType::I32 => "int32_t",
        LeafType::U32 => "uint32_t",
        LeafType::I64 => "int64_t",
        LeafType::U64 => "uint64_t",
        LeafType::F32 => "float",
        LeafType::F64 => "double",
        LeafType::Str => "char*",
        _ => "?",
    }
}

/// Decode ROOT's `fCompress` (`algorithm * 100 + level`) into a label.
pub fn compression_label(compress: u32) -> String {
    use oxiroot::Compression;
    match Compression::try_from(compress) {
        Ok(Compression::Stored) => "none".to_string(),
        Ok(Compression::Zlib(level)) => format!("zlib (level {level})"),
        Ok(Compression::Lzma(level)) => format!("lzma (level {level})"),
        Ok(Compression::Lz4(level)) => format!("lz4 (level {level})"),
        Ok(Compression::Zstd(level)) => format!("zstd (level {level})"),
        // `Compression` can grow an algorithm this label does not name yet.
        Ok(_) | Err(_) => format!("unknown (setting {compress})"),
    }
}

/// Format a ROOT file-version integer (`62400`) as `6.24/00`.
pub fn root_version(v: u32) -> String {
    format!("{}.{:02}/{:02}", v / 10000, (v / 100) % 100, v % 100)
}

/// A left/right-aligned column table with a header row.
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    right: Vec<bool>,
}

impl Table {
    /// A new table with these column headers (all left-aligned by default).
    pub fn new(headers: &[&str]) -> Self {
        Self {
            right: vec![false; headers.len()],
            headers: headers
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            rows: Vec::new(),
        }
    }

    /// Right-align the given column indices (for numbers).
    pub fn right_align(mut self, cols: &[usize]) -> Self {
        for &c in cols {
            if let Some(slot) = self.right.get_mut(c) {
                *slot = true;
            }
        }
        self
    }

    /// Append a row (cell count should match the header count).
    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    /// The table as lines, each column padded to its widest cell.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let ncol = self.headers.len();
        let mut width = vec![0usize; ncol];
        for (c, h) in self.headers.iter().enumerate() {
            width[c] = h.chars().count();
        }
        for row in &self.rows {
            for (c, cell) in row.iter().enumerate().take(ncol) {
                width[c] = width[c].max(cell.chars().count());
            }
        }
        let mut line = |cells: &[String]| {
            let parts: Vec<String> = (0..ncol)
                .map(|c| {
                    let cell = cells.get(c).map_or("", String::as_str);
                    pad(cell, width[c], self.right[c])
                })
                .collect();
            out.push_str(parts.join("  ").trim_end());
            out.push('\n');
        };
        line(&self.headers);
        for row in &self.rows {
            line(row);
        }
        out
    }
}

/// Pad `s` to `width` columns, on the left when `right` (right-aligned).
fn pad(s: &str, width: usize, right: bool) -> String {
    let n = s.chars().count();
    let fill = " ".repeat(width.saturating_sub(n));
    if right {
        format!("{fill}{s}")
    } else {
        format!("{s}{fill}")
    }
}

#[cfg(test)]
mod tests {
    use super::compression_label;

    #[test]
    fn compression_label_names_the_algorithm_and_level() {
        assert_eq!(compression_label(0), "none");
        assert_eq!(compression_label(101), "zlib (level 1)");
        assert_eq!(compression_label(505), "zstd (level 5)");
        assert_eq!(compression_label(404), "lz4 (level 4)");
        assert_eq!(compression_label(207), "lzma (level 7)");
        assert_eq!(compression_label(905), "unknown (setting 905)");
    }
}
