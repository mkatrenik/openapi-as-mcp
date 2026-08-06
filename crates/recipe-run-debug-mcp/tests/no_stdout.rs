//! Guard: on stdio transport, stdout is the JSON-RPC channel. A `println!` anywhere in the server
//! corrupts every session, and the failure looks like a client bug, so catch it here.

use std::fs;
use std::path::Path;

#[test]
fn no_source_file_writes_to_stdout() {
    let mut offenders = Vec::new();
    scan(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src").as_path(),
        &mut offenders,
    );

    assert!(
        offenders.is_empty(),
        "these write to stdout, which corrupts the MCP protocol stream — use tracing instead:\n{}",
        offenders.join("\n")
    );
}

/// Net brace balance of a line. Good enough for well-formatted Rust; braces inside string
/// literals would skew it, but the sources this guards are `cargo fmt`-clean.
fn brace_delta(line: &str) -> i32 {
    line.chars().filter(|c| *c == '{').count() as i32
        - line.chars().filter(|c| *c == '}').count() as i32
}

fn scan(dir: &Path, offenders: &mut Vec<String>) {
    let entries = fs::read_dir(dir).expect("readable src directory");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, offenders);
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        // The CLI owns stdout by design: in CLI mode there is no JSON-RPC stream to corrupt, and
        // the server path never reaches this module. It is the one place printing is correct.
        if path.file_name().is_some_and(|name| name == "cli.rs") {
            continue;
        }

        let content = fs::read_to_string(&path).expect("readable source file");

        // Test modules may print freely (cargo captures their stdout), so skip exactly the
        // `#[cfg(test)]` block — tracked by brace depth, because code can follow it in the file.
        let mut pending_test_attr = false;
        let mut test_depth: Option<i32> = None;

        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();

            if let Some(depth) = test_depth.as_mut() {
                *depth += brace_delta(line);
                if *depth <= 0 {
                    test_depth = None;
                }
                continue;
            }

            if trimmed.starts_with("#[cfg(test)]") {
                pending_test_attr = true;
                continue;
            }
            if pending_test_attr {
                let delta = brace_delta(line);
                if delta > 0 {
                    test_depth = Some(delta);
                    pending_test_attr = false;
                    continue;
                }
                // Attribute on a plain item (e.g. a single `#[cfg(test)] fn`): nothing to skip.
                pending_test_attr = false;
            }

            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("println!") || trimmed.contains("print!") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    idx + 1,
                    trimmed.trim()
                ));
            }
        }
    }
}
