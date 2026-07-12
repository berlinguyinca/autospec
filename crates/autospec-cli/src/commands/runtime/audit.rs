use autospec_core::runtime_policy::{classify_path, is_supported_runtime_path, RuntimeClass};
use std::fs;
use std::path::{Path, PathBuf};

const PLATFORM_ROOTS: &[&str] = &["scripts", "skills", "packages"];
const SKIPPED_DIRECTORIES: &[&str] = &[".git", "target", "node_modules"];
const CLASSES: &[RuntimeClass] = &[
    RuntimeClass::R0,
    RuntimeClass::R1,
    RuntimeClass::R2,
    RuntimeClass::R3,
    RuntimeClass::R4,
];

#[derive(Default)]
struct AuditGroups {
    r0: Vec<String>,
    r1: Vec<String>,
    r2: Vec<String>,
    r3: Vec<String>,
    r4: Vec<String>,
}

impl AuditGroups {
    fn add(&mut self, class: RuntimeClass, path: String) {
        match class {
            RuntimeClass::R0 => self.r0.push(path),
            RuntimeClass::R1 => self.r1.push(path),
            RuntimeClass::R2 => self.r2.push(path),
            RuntimeClass::R3 => self.r3.push(path),
            RuntimeClass::R4 => self.r4.push(path),
        }
    }

    fn paths(&self, class: RuntimeClass) -> &[String] {
        match class {
            RuntimeClass::R0 => &self.r0,
            RuntimeClass::R1 => &self.r1,
            RuntimeClass::R2 => &self.r2,
            RuntimeClass::R3 => &self.r3,
            RuntimeClass::R4 => &self.r4,
        }
    }
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let (root, json) = parse_args(args)?;
    if !root.exists() {
        return Err(format!(
            "runtime audit root does not exist: {}",
            root.display()
        ));
    }
    if !root.is_dir() {
        return Err(format!(
            "runtime audit root is not a directory: {}",
            root.display()
        ));
    }

    let groups = audit_root(&root)?;
    if json {
        print_json(&root, &groups);
    } else {
        print_text(&root, &groups);
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<(PathBuf, bool), String> {
    let mut root = std::env::current_dir()
        .map_err(|error| format!("failed to resolve current directory: {error}"))?;
    let mut json = false;
    let mut index = 0;

    while let Some(argument) = args.get(index) {
        match argument.as_str() {
            "--json" => {
                json = true;
                index += 1;
            }
            "--root" => {
                let path = args
                    .get(index + 1)
                    .ok_or_else(|| "runtime audit --root requires a path".to_string())?;
                root = PathBuf::from(path);
                index += 2;
            }
            unknown => {
                return Err(format!("unknown autospec runtime audit option: {unknown}"));
            }
        }
    }

    Ok((root, json))
}

fn audit_root(root: &Path) -> Result<AuditGroups, String> {
    let mut paths = Vec::new();
    for platform_root in PLATFORM_ROOTS {
        let directory = root.join(platform_root);
        if is_symlink(&directory)? {
            continue;
        }
        collect_files(&directory, root, &mut paths)?;
    }
    paths.sort();

    let mut groups = AuditGroups::default();
    for path in paths {
        let verdict = classify_path(&path);
        groups.add(verdict.class, verdict.path);
    }
    Ok(groups)
}

fn collect_files(directory: &Path, root: &Path, paths: &mut Vec<String>) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;

        if file_type.is_dir() {
            if should_skip_directory(&path) {
                continue;
            }
            collect_files(&path, root, paths)?;
        } else if file_type.is_file() {
            let relative_path = repository_relative_path(&path, root)?;
            if is_supported_runtime_path(&relative_path) {
                paths.push(relative_path);
            }
        }
    }
    Ok(())
}

fn should_skip_directory(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| SKIPPED_DIRECTORIES.contains(&name))
}

fn is_symlink(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

fn repository_relative_path(path: &Path, root: &Path) -> Result<String, String> {
    path.strip_prefix(root)
        .map_err(|_| format!("{} is outside {}", path.display(), root.display()))
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn print_text(root: &Path, groups: &AuditGroups) {
    println!("runtime audit: {}", root.display());
    for class in CLASSES {
        let paths = groups.paths(*class);
        println!("{} ({})", class.as_str(), paths.len());
        for path in paths {
            println!("  {path}");
        }
    }
}

fn print_json(root: &Path, groups: &AuditGroups) {
    let classes = CLASSES
        .iter()
        .map(|class| {
            format!(
                "\"{}\":[{}]",
                class.as_str(),
                groups
                    .paths(*class)
                    .iter()
                    .map(|path| format!("\"{}\"", super::escape_json(path)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    println!(
        "{{\"command\":\"runtime audit\",\"root\":\"{}\",\"classes\":{{{classes}}}}}",
        super::escape_json(&root.to_string_lossy())
    );
}
