use std::fs;
use std::path::Path;

/// Validate println/eprintln call sites against the targets declared by each crate.
/// The check is intentionally dependency-free: Cargo manifests are parsed only for
/// target declarations, while source inspection is limited to Rust macro call lines.
pub fn validate(root: &Path) -> Result<(), String> {
    let crates = root.join("crates");
    if !crates.is_dir() {
        return Ok(());
    }
    let mut findings = Vec::new();
    for entry in fs::read_dir(crates).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest = path.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let manifest_text = fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
        let has_lib = manifest_text.lines().any(|line| line.trim() == "[lib]");
        let has_bin = manifest_text.lines().any(|line| line.trim() == "[[bin]]")
            || path.join("src/main.rs").is_file();
        let target_kind = match (has_lib, has_bin) {
            (true, true) => "mixed",
            (true, false) => "library",
            (false, true) => "binary",
            (false, false) => continue,
        };
        scan_sources(&path.join("src"), target_kind, root, &mut findings)?;
    }
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings.join("\n"))
    }
}

fn scan_sources(
    path: &Path,
    target_kind: &str,
    root: &Path,
    findings: &mut Vec<String>,
) -> Result<(), String> {
    if !path.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let file = entry.path();
        if file.is_dir() {
            scan_sources(&file, target_kind, root, findings)?;
        } else if file.extension().and_then(|x| x.to_str()) == Some("rs") {
            let text = fs::read_to_string(&file).map_err(|e| e.to_string())?;
            // Name the path relative to the validation root: the reason doubles as a
            // comparison key for machine consumers, and an absolute path would embed the
            // worktree location, so the same tree validated from two directories would
            // produce different bytes (#3802).
            let display = file.strip_prefix(root).unwrap_or(&file);
            let lines: Vec<&str> = text.lines().collect();
            for (line_no, line) in lines.iter().enumerate() {
                if !(line.contains("println!(") || line.contains("eprintln!("))
                // autospec:allow-output
                {
                    continue;
                }
                // The escape marker may sit inside the invocation rather than on
                // the line that opens it. A call long enough to need wrapping
                // puts its first argument -- and any leading comment -- on the
                // following line, which is exactly where rustfmt moves it. Three
                // legitimate test-skip notices were being reported as violations
                // for that reason alone, so the escape hatch failed precisely
                // when the call was long enough to want it.
                //
                // Bounded to the invocation's first two lines rather than
                // scanning to the closing paren: a marker further inside would be
                // describing an argument, not the call.
                let marked = line.contains("autospec:allow-output")
                    || lines
                        .get(line_no + 1)
                        .is_some_and(|next| next.contains("autospec:allow-output"));
                if marked {
                    continue;
                }
                if target_kind == "binary" {
                    continue;
                }
                let remediation = if target_kind == "library" {
                    "use tracing or an injected writer"
                } else {
                    "add // autospec:allow-output or use an injected writer"
                };
                findings.push(format!(
                    "output-macro: target_kind={target_kind} file={} line={} remediation={remediation}",
                    display.display(), line_no + 1
                ));
            }
        }
    }
    Ok(())
}
