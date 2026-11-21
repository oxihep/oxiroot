//! A tiny dependency-free JSON writer for the `--json` output mode.

use std::fmt::Write as _;

/// A JSON value the commands build up and then render compactly. Non-finite
/// floats (`NaN`/`inf`) render as `null`, since JSON has no representation for
/// them.
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    F64(f64),
    Str(String),
    Array(Vec<Self>),
    Object(Vec<(&'static str, Self)>),
}

impl Json {
    /// A string value from anything `Into<String>`.
    pub fn s(value: impl Into<String>) -> Self {
        Self::Str(value.into())
    }

    /// Render as a compact, single-line JSON string.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Int(n) => out.push_str(&n.to_string()),
            Self::F64(x) if x.is_finite() => out.push_str(&x.to_string()),
            Self::F64(_) => out.push_str("null"),
            Self::Str(s) => escape(s, out),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    escape(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Append `s` as a quoted, escaped JSON string.
fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                // Writing to a `String` cannot fail.
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
