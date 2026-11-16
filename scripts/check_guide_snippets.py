#!/usr/bin/env python3
"""Compile every ```rust block of the guides (docs/**/*.md) and the README.

Each page's blocks, in order, become the body of one function in a throwaway
crate that depends on the workspace's `oxiroot` with its features, with the
prelude in scope and `?` allowed (a block may use what an earlier block on the
same page defined, as a reader would):

    fn page() -> Result<(), Box<dyn std::error::Error>> { <blocks>; Ok(()) }

Lines starting with `# ` are rustdoc-hidden lines and are kept as code.

Blocks are compiled, not run (they open files that do not exist). A block that
is deliberately a fragment (it works on the reader's own `data`, say) opts out
with an HTML comment on the line before its fence, invisible on the site:

    <!-- fragment -->

Usage: python3 scripts/check_guide_snippets.py [--keep DIR]
Exit status 1 lists every block that does not compile, as file:line.
"""
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FENCE = re.compile(r"^([ \t]*)```rust([^\n]*)\n(.*?)^\1```", re.S | re.M)


def blocks():
    files = sorted(glob.glob(os.path.join(ROOT, "docs", "**", "*.md"), recursive=True))
    files.append(os.path.join(ROOT, "README.md"))
    for path in files:
        text = open(path, encoding="utf-8").read()
        for m in FENCE.finditer(text):
            before = text[: m.start()].rstrip().rsplit("\n", 1)[-1].strip()
            if before == "<!-- fragment -->" or "ignore" in m.group(2):
                continue
            indent = m.group(1)
            lines = [
                line[len(indent):] if line.startswith(indent) else line
                for line in m.group(3).splitlines()
            ]
            # rustdoc's hidden lines: `# code` is code, a lone `#` is blank.
            body = "\n".join(
                l[2:] if l.startswith("# ") else ("" if l.strip() == "#" else l) for l in lines
            )
            line = text.count("\n", 0, m.start()) + 1
            yield os.path.relpath(path, ROOT), line, body


def main():
    keep = sys.argv[2] if len(sys.argv) > 2 and sys.argv[1] == "--keep" else None
    work = keep or tempfile.mkdtemp(prefix="guide-snippets-")
    src = os.path.join(work, "src")
    os.makedirs(src, exist_ok=True)
    open(os.path.join(work, "Cargo.toml"), "w").write(
        "[package]\nname = \"guide-snippets\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n"
        "[dependencies]\n"
        f"oxiroot = {{ path = \"{os.path.join(ROOT, 'crates', 'oxiroot')}\", features = [\"plot\", \"fit\", \"rayon\", \"mmap\", \"http\", \"xrootd\"] }}\n\n"
        "[workspace]\n"
    )
    if os.path.exists(os.path.join(ROOT, "Cargo.lock")):
        shutil.copy(os.path.join(ROOT, "Cargo.lock"), os.path.join(work, "Cargo.lock"))
    index = []  # (path, first line) of every block, by its line in lib.rs
    lib = ["#![allow(unused, clippy::all, unused_must_use, unreachable_code, dead_code)]"]
    pages = {}
    for path, line, body in blocks():
        pages.setdefault(path, []).append((line, body))
    for k, (path, items) in enumerate(pages.items()):
        lib.append(f"mod p{k} {{\n    use oxiroot::prelude::*;")
        lib.append("    pub fn page() -> Result<(), Box<dyn std::error::Error>> {")
        # Each block opens a scope that stays open to the end of the page: it
        # sees what earlier blocks defined, and may repeat their `use` lines.
        for line, body in items:
            lib.append(f"// BLOCK {path}:{line}")
            lib.append("{")
            lib.extend(body.splitlines())
            lib.append(";")
        lib.append("}" * len(items))
        lib.append("    Ok(())\n    }\n}")
    open(os.path.join(src, "lib.rs"), "w").write("\n".join(lib) + "\n")
    out = subprocess.run(
        ["cargo", "check", "--quiet", "--message-format=short"],
        cwd=work,
        capture_output=True,
        text=True,
    )
    bad = {}
    for msg in out.stderr.splitlines():
        m = re.match(r"src/lib\.rs:(\d+):\d+: error", msg)
        if not m:
            continue
        lineno = int(m.group(1))
        # Find the block this line belongs to.
        src_lines = open(os.path.join(src, "lib.rs")).read().splitlines()
        for i in range(lineno - 1, -1, -1):
            mm = re.match(r"// BLOCK (.+):(\d+)$", src_lines[i])
            if mm:
                bad.setdefault((mm.group(1), int(mm.group(2))), []).append(
                    msg.split(": error", 1)[1].strip()
                )
                break
    if not keep:
        shutil.rmtree(work, ignore_errors=True)
    nblocks = sum(len(v) for v in pages.values())
    print(f"{nblocks} blocks on {len(pages)} pages checked, {len(bad)} do not compile")
    for (path, line), errs in sorted(bad.items()):
        print(f"  {path}:{line}: {errs[0]}" + (f" (+{len(errs) - 1} more)" if len(errs) > 1 else ""))
    if out.returncode != 0 and not bad:
        print(out.stderr[-3000:])
    return 1 if bad or out.returncode != 0 else 0


if __name__ == "__main__":
    sys.exit(main())
