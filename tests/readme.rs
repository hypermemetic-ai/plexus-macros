//! The pages' code blocks are files of this repository.
//!
//! A fenced block is named by the comment above it,
//! `<!-- examples/counter/src/lib.rs -->`, and is lines that stand together
//! in that file, at any indentation. A block that differs from its file
//! fails this test; so does a file that is gone. Paths are from the
//! repository's root.
//!
//! In `README.md` and `plexus-client/README.md` every block names a file. In
//! `docs/*.md` a block with no comment above it is not checked.
//!
//! A manifest is quoted as a crate outside this repository writes it: a
//! dependency's `path` is left out, and `{ version = "0.11" }` is `"0.11"`.

use std::fs;
use std::path::Path;

struct Block {
    /// The file the comment above the block names, if one does.
    source: Option<String>,
    /// The index of the block's first line, and of its closing fence.
    lines: std::ops::Range<usize>,
}

fn blocks(page: &str, text: &[&str]) -> Vec<Block> {
    let mut found = Vec::new();
    let mut above = "";
    let mut index = 0;
    while index < text.len() {
        let line = text[index];
        if line.starts_with("```") {
            let source = above.strip_prefix("<!-- ").and_then(|rest| rest.strip_suffix(" -->")).map(str::to_string);
            let close = (index + 1..text.len())
                .find(|&i| text[i].starts_with("```"))
                .unwrap_or_else(|| panic!("{page}:{}: the block is not closed", index + 1));
            found.push(Block { source, lines: index + 1..close });
            index = close;
            above = "";
        } else if !line.trim().is_empty() {
            above = line;
        }
        index += 1;
    }
    found
}

/// The lines without their common indentation and trailing spaces.
fn dedented(lines: &[&str]) -> Vec<String> {
    let indent = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    lines.iter().map(|l| l.get(indent..).unwrap_or("").trim_end().to_string()).collect()
}

/// A manifest's line as a crate outside this repository writes it.
fn published(line: &str) -> String {
    let Some(at) = line.find("path = \"") else { return line.to_string() };
    let end = at + "path = \"".len() + line[at + "path = \"".len()..].find('"').expect("a closed path") + 1;
    let (before, after) = (&line[..at], &line[end..]);
    let line = match (before.trim_end().strip_suffix(','), after.strip_prefix(", ")) {
        (_, Some(after)) => format!("{before}{after}"),
        (Some(before), None) => format!("{before}{after}"),
        (None, None) => format!("{before}{after}"),
    };
    match line.split_once(" = { version = ") {
        Some((name, rest)) if rest.trim_end().ends_with("\" }") && !rest.contains(',') => {
            format!("{name} = {}", rest.trim_end().trim_end_matches(" }"))
        }
        _ => line,
    }
}

fn wrong_blocks(root: &Path, page: &str, every_block_names_a_file: bool) -> Vec<String> {
    let text = fs::read_to_string(root.join(page)).unwrap_or_else(|e| panic!("{page}: {e}"));
    let text: Vec<&str> = text.lines().collect();
    let mut wrong = Vec::new();
    for block in blocks(page, &text) {
        let at = block.lines.start;
        let Some(source) = &block.source else {
            if every_block_names_a_file {
                wrong.push(format!("{page}:{at}: the block names no file in a comment above it"));
            }
            continue;
        };
        let Ok(file) = fs::read_to_string(root.join(source)) else {
            wrong.push(format!("{page}:{at}: {source} cannot be read"));
            continue;
        };
        let file: Vec<String> = if source.ends_with(".toml") { file.lines().map(published).collect() } else { file.lines().map(str::to_string).collect() };
        let file: Vec<&str> = file.iter().map(String::as_str).collect();
        let quoted = dedented(&text[block.lines.clone()]);
        if quoted.is_empty() || !file.windows(quoted.len()).any(|window| dedented(window) == quoted) {
            wrong.push(format!("{page}:{at}: not what {source} says"));
        }
    }
    wrong
}

#[test]
fn every_block_is_its_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut wrong = Vec::new();
    for readme in ["README.md", "plexus-client/README.md"] {
        wrong.extend(wrong_blocks(root, readme, true));
    }
    let mut pages: Vec<String> = fs::read_dir(root.join("docs"))
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".md"))
        .map(|name| format!("docs/{name}"))
        .collect();
    pages.sort();
    for page in &pages {
        wrong.extend(wrong_blocks(root, page, false));
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_manifest_is_quoted_without_its_paths() {
    assert_eq!(published(r#"plexus-macros = { version = "0.11", path = "../.." }"#), r#"plexus-macros = "0.11""#);
    assert_eq!(
        published(r#"tokio = { version = "1.0", path = "../tokio", features = ["rt"] }"#),
        r#"tokio = { version = "1.0", features = ["rt"] }"#
    );
    assert_eq!(published(r#"serde = { version = "1.0", features = ["derive"] }"#), r#"serde = { version = "1.0", features = ["derive"] }"#);
    assert_eq!(published(r#"futures = "0.3""#), r#"futures = "0.3""#);
}
