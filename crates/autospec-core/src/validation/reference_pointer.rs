//! Reference-pointer integrity gate.
//!
//! Every `**MUST** read `<path>`` pointer in a trio body must resolve to an
//! existing, non-stub file; if the pointer names a `(section "X")`, that heading
//! must be present in the target. This keeps reference extraction honest: a
//! dangling or empty target would silently strand the content the pointer claims
//! to hold, and it stops a one-line body anchor from being the only thing a gate
//! can see once the real procedure moves to a reference.
//!
//! Resolving in the repository is necessary but not sufficient: the file has to
//! reach the machine that runs the skill. So this also holds each skill's
//! `install.sh` to shipping every file under its `references/` directory. Before
//! #3228 no installer shipped that directory at all, and an installed skill run
//! against a target repository had nothing to resolve — a `**MUST** read` phase
//! became unreachable with no error anywhere.

use std::fs;
use std::path::Path;

use super::structural::skill_directories;

const MIN_REFERENCE_BYTES: u64 = 200;

struct MustReadPointer {
    path: String,
    section: Option<String>,
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

pub fn validate(root: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    for skill_dir in skill_directories(root)? {
        errors.extend(check_member_pointers(root, &skill_dir));
        errors.extend(check_reference_shipping(root, &skill_dir));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn check_member_pointers(root: &Path, skill_dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    for member in ["SKILL.md", "codex/prompt.md", "opencode/agent.md"] {
        let path = skill_dir.join(member);
        let Ok(document) = read(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        for pointer in must_read_pointers(&document) {
            if let Some(error) = check_reference_pointer(root, &rel, &pointer) {
                errors.push(error);
            }
        }
    }
    errors
}

fn check_reference_pointer(root: &Path, rel: &str, pointer: &MustReadPointer) -> Option<String> {
    let target = root.join(&pointer.path);
    if !target.is_file() {
        return Some(format!(
            "{rel}: `**MUST** read` pointer to `{}` dangles (file missing)",
            pointer.path
        ));
    }
    let bytes = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    if bytes < MIN_REFERENCE_BYTES {
        return Some(format!(
            "{rel}: `**MUST** read` pointer to `{}` resolves to a stub ({} bytes < {})",
            pointer.path, bytes, MIN_REFERENCE_BYTES
        ));
    }
    let Ok(target_doc) = read(&target) else {
        return None;
    };
    if let Some(section) = &pointer.section {
        if !target_doc.contains(section.as_str()) {
            return Some(format!(
                "{rel}: `**MUST** read` pointer to `{}` claims section `{}` but it is absent",
                pointer.path, section
            ));
        }
    }
    None
}

/// Collect every non-hidden file under `dir`, as paths relative to it. Hidden
/// entries are placeholders such as `.gitkeep`, which nothing reads.
fn reference_files(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    names.sort();
    for path in names {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let rel = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            reference_files(&path, &rel, out);
        } else {
            out.push(rel);
        }
    }
}

/// Hold the skill's own installer to shipping every reference file. A skill with
/// no `install.sh` has no shipping contract to check (the validator's own
/// fixtures are in that shape).
fn check_reference_shipping(root: &Path, skill_dir: &Path) -> Vec<String> {
    let installer = skill_dir.join("install.sh");
    let references = skill_dir.join("references");
    if !installer.is_file() || !references.is_dir() {
        return Vec::new();
    }
    let mut files = Vec::new();
    reference_files(&references, "", &mut files);
    if files.is_empty() {
        return Vec::new();
    }
    let Ok(script) = read(&installer) else {
        return Vec::new();
    };
    let declared = script
        .lines()
        .find_map(|line| line.strip_prefix("SKILL_REFERENCE_FILES=\""))
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let rel = installer
        .strip_prefix(root)
        .unwrap_or(&installer)
        .display()
        .to_string();
    files
        .into_iter()
        .filter(|file| !declared.iter().any(|d| d == file))
        .map(|file| {
            format!("{rel}: SKILL_REFERENCE_FILES does not ship references/{file}, so an installed skill cannot resolve it")
        })
        .collect()
}

/// Extract `**MUST** read `<path>`` pointers (optionally naming a `(section "X")`)
/// from a document. Paths are resolved against the repo root by the caller.
fn must_read_pointers(document: &str) -> Vec<MustReadPointer> {
    const MARKER: &str = "**MUST** read `";
    const SECTION_MARKER: &str = "(section \"";
    let mut out = Vec::new();
    for line in document.lines() {
        let Some(start) = line.find(MARKER) else {
            continue;
        };
        let after = &line[start + MARKER.len()..];
        let Some(close) = after.find('`') else {
            continue;
        };
        let path = after[..close].to_string();
        if path.is_empty() {
            continue;
        }
        let section = after[close..]
            .find(SECTION_MARKER)
            .map(|i| &after[close + i + SECTION_MARKER.len()..])
            .and_then(|rest| rest.find('"').map(|j| rest[..j].to_string()));
        out.push(MustReadPointer { path, section });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("autospec-{name}-{nonce}"));
        fs::create_dir_all(root.join("skills")).expect("skills dir");
        root
    }

    fn write_pointer_trio(root: &Path, skill: &str, body: &str) {
        let dir = root.join("skills").join(skill);
        fs::create_dir_all(dir.join("codex")).expect("codex dir");
        fs::create_dir_all(dir.join("opencode")).expect("opencode dir");
        fs::write(dir.join("SKILL.md"), body).expect("skill fixture");
        fs::write(dir.join("codex/prompt.md"), body).expect("codex fixture");
        fs::write(dir.join("opencode/agent.md"), body).expect("opencode fixture");
    }

    #[test]
    fn reference_pointer_integrity_passes_for_resolving_pointer() {
        let root = temp_root("ref-ptr-valid");
        let body = "**MUST** read `skills/demo/references/guide.md` (section \"## Setup\") and follow it.\n";
        write_pointer_trio(&root, "demo", body);
        let refdir = root.join("skills/demo/references");
        fs::create_dir_all(&refdir).expect("reference dir");
        fs::write(refdir.join("guide.md"), "## Setup\n\n".repeat(20)).expect("reference");
        validate(&root).expect("resolving pointer with claimed section passes");
    }

    #[test]
    fn reference_pointer_integrity_fails_on_dangling_pointer() {
        let root = temp_root("ref-ptr-dangle");
        let body = "**MUST** read `skills/demo/references/missing.md` and follow it.\n";
        write_pointer_trio(&root, "demo", body);
        let err = validate(&root).expect_err("dangling pointer must fail");
        assert!(err.contains("dangles"), "unexpected: {err}");
    }

    #[test]
    fn reference_pointer_integrity_fails_on_stub_target() {
        let root = temp_root("ref-ptr-stub");
        let body = "**MUST** read `skills/demo/references/stub.md` and follow it.\n";
        write_pointer_trio(&root, "demo", body);
        let refdir = root.join("skills/demo/references");
        fs::create_dir_all(&refdir).expect("reference dir");
        fs::write(refdir.join("stub.md"), "tiny\n").expect("stub reference");
        let err = validate(&root).expect_err("stub target must fail");
        assert!(err.contains("stub"), "unexpected: {err}");
    }

    fn write_installer(root: &Path, skill: &str, declared: &str) {
        let dir = root.join("skills").join(skill);
        fs::create_dir_all(&dir).expect("skill dir");
        fs::write(
            dir.join("install.sh"),
            format!("#!/bin/sh\nSKILL_REFERENCE_FILES=\"{declared}\"\n"),
        )
        .expect("installer fixture");
    }

    #[test]
    fn reference_shipping_passes_when_the_installer_declares_every_file() {
        let root = temp_root("ref-ship-ok");
        write_pointer_trio(&root, "demo", "no pointers here\n");
        let refdir = root.join("skills/demo/references");
        fs::create_dir_all(refdir.join("nested")).expect("reference dir");
        fs::write(refdir.join("guide.md"), "body\n").expect("reference");
        fs::write(refdir.join("nested/deep.md"), "body\n").expect("nested reference");
        // A placeholder nothing reads must not be demanded of the installer.
        fs::write(refdir.join(".gitkeep"), "").expect("placeholder");
        write_installer(&root, "demo", "guide.md nested/deep.md");
        validate(&root).expect("fully declared references pass");
    }

    #[test]
    fn reference_shipping_fails_when_the_installer_omits_a_file() {
        let root = temp_root("ref-ship-missing");
        write_pointer_trio(&root, "demo", "no pointers here\n");
        let refdir = root.join("skills/demo/references");
        fs::create_dir_all(&refdir).expect("reference dir");
        fs::write(refdir.join("guide.md"), "body\n").expect("reference");
        fs::write(refdir.join("forgotten.md"), "body\n").expect("reference");
        write_installer(&root, "demo", "guide.md");
        let err = validate(&root).expect_err("an unshipped reference must fail");
        assert!(err.contains("forgotten.md"), "unexpected: {err}");
        assert!(!err.contains("guide.md"), "declared file flagged: {err}");
    }

    #[test]
    fn reference_shipping_fails_when_the_installer_declares_nothing() {
        let root = temp_root("ref-ship-empty");
        write_pointer_trio(&root, "demo", "no pointers here\n");
        let refdir = root.join("skills/demo/references");
        fs::create_dir_all(&refdir).expect("reference dir");
        fs::write(refdir.join("guide.md"), "body\n").expect("reference");
        let dir = root.join("skills/demo");
        fs::write(dir.join("install.sh"), "#!/bin/sh\necho no declaration\n")
            .expect("installer fixture");
        let err = validate(&root).expect_err("a missing declaration must fail");
        assert!(err.contains("guide.md"), "unexpected: {err}");
    }

    #[test]
    fn reference_pointer_integrity_fails_when_claimed_section_absent() {
        let root = temp_root("ref-ptr-nosection");
        let body = "**MUST** read `skills/demo/references/guide.md` (section \"## Setup\") and follow it.\n";
        write_pointer_trio(&root, "demo", body);
        let refdir = root.join("skills/demo/references");
        fs::create_dir_all(&refdir).expect("reference dir");
        fs::write(refdir.join("guide.md"), "## Other\n\n".repeat(20)).expect("reference");
        let err = validate(&root).expect_err("missing claimed section must fail");
        assert!(err.contains("claims section"), "unexpected: {err}");
    }
}
