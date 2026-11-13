//! No function of this workspace answers `Result<_, String>`: an error is a type a caller can match (krizaka-internal#2959).
//! The scan reads the sources of the root crate and of the plugin, tests included.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// True when the line spells a result whose error is a bare `String` (`Result<T, String>`, `type Err = String`).
fn is_string_error(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or_default();
    let in_result = code
        .match_indices("Result<")
        .any(|(at, _)| code[at..].contains(", String>") || code[at..].contains(",String>"));
    in_result || code.contains("type Err = String")
}

#[test]
fn the_scan_knows_what_it_looks_for() {
    assert!(is_string_error("fn a() -> Result<(), String> {"));
    assert!(is_string_error("type R = Result<Vec<u8>, String>;"));
    assert!(is_string_error("    type Err = String;"));
    assert!(!is_string_error("fn a() -> Result<String, Failure> {"));
    assert!(!is_string_error("fn a() -> Result<(), SecretError> {"));
    assert!(!is_string_error("// Result<(), String> was the old shape"));
}

#[test]
fn no_signature_answers_a_string_error() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("plugin").join("src"), &mut files);
    assert!(files.len() > 10, "the scan found no sources");
    let mut found = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap_or_default();
        for (number, line) in text.lines().enumerate() {
            if is_string_error(line) {
                let shown = file.strip_prefix(root).unwrap_or(&file);
                found.push(format!(
                    "{}:{}: {}",
                    shown.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        found.is_empty(),
        "string errors left:\n{}",
        found.join("\n")
    );
}
