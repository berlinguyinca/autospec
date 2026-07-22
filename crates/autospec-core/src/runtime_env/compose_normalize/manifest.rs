use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use yaml_edit::{Document, Mapping};

use super::PlannedFile;
use crate::runtime_env::{
    ComposeExport, ExportProtocol, ExportValue, RuntimeEnvError, RuntimeManifest, RuntimeResources,
};

pub(super) fn load_or_default(repo: &Path) -> Result<RuntimeManifest, RuntimeEnvError> {
    let autospec = repo.join(".autospec/runtime.yml");
    let agent = repo.join(".agent-runtime.yml");
    if std::fs::symlink_metadata(&autospec).is_ok() || std::fs::symlink_metadata(&agent).is_ok() {
        return RuntimeManifest::read_from_repo(repo);
    }
    Ok(RuntimeManifest {
        path: autospec,
        name: None,
        version: 2,
        default_mode: None,
        modes: Vec::new(),
        resources: RuntimeResources::default(),
    })
}

pub(super) fn canonical_path(manifest: &RuntimeManifest) -> Result<PathBuf, RuntimeEnvError> {
    Ok(manifest.path().to_path_buf())
}

pub(super) fn render(
    files: &mut [PlannedFile],
    manifest_path: &Path,
    manifest: &RuntimeManifest,
    repo: &Path,
    compose_files: &[PathBuf],
    exports: &[ComposeExport],
) -> Result<(), RuntimeEnvError> {
    let file = files
        .iter_mut()
        .find(|file| file.path == manifest_path)
        .ok_or_else(|| RuntimeEnvError::new("runtime manifest was not planned"))?;
    if file.identity.is_none() {
        return render_new(file, repo, compose_files, exports);
    }
    let text = std::str::from_utf8(&file.original)
        .map_err(|_| RuntimeEnvError::new("runtime manifest must be UTF-8"))?;
    let root = parse_v2_root(text)?;
    let missing_files = missing_files(manifest, repo, compose_files);
    let missing_exports = exports
        .iter()
        .filter(|export| !manifest.resources().compose.exports.contains(export))
        .collect::<Vec<_>>();
    if missing_files.is_empty() && missing_exports.is_empty() {
        return validate_rendered(file);
    }
    file.rendered = apply_byte_edits(
        text,
        manifest_edits(text, &root, &missing_files, &missing_exports)?,
    );
    validate_rendered(file)
}

fn render_new(
    file: &mut PlannedFile,
    repo: &Path,
    compose_files: &[PathBuf],
    exports: &[ComposeExport],
) -> Result<(), RuntimeEnvError> {
    let files = compose_files
        .iter()
        .map(|path| path.strip_prefix(repo).unwrap_or(path).to_path_buf())
        .collect::<Vec<_>>();
    let exports = exports.iter().collect::<Vec<_>>();
    file.rendered = format!(
        "version: 2\nresources:\n{}",
        compose_block(2, &files, &exports)
    )
    .into_bytes();
    validate_rendered(file)
}

/// Runtime manifests use POSIX-style logical paths regardless of the host OS.
/// `Path::display` emits backslashes on Windows, which would make otherwise
/// identical migrations differ between worktrees on different platforms.
fn logical_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn validate_rendered(file: &PlannedFile) -> Result<(), RuntimeEnvError> {
    let text = std::str::from_utf8(&file.rendered)
        .map_err(|_| RuntimeEnvError::new("rendered runtime manifest must be UTF-8"))?;
    RuntimeManifest::parse(text).map(|_| ())
}

pub(super) fn flow_mapping_unsupported(text: &str) -> Result<bool, RuntimeEnvError> {
    let root = parse_v2_root(text)?;
    if mapping_is_flow(text, &root) {
        return Ok(true);
    }
    let Some(resources) = root.get_mapping("resources") else {
        return Ok(false);
    };
    if mapping_is_flow(text, &resources) {
        return Ok(true);
    }
    Ok(resources
        .get_mapping("compose")
        .is_some_and(|compose| mapping_is_flow(text, &compose)))
}

fn mapping_is_flow(source: &str, mapping: &Mapping) -> bool {
    let range = mapping.byte_range();
    source[range.start as usize..range.end as usize]
        .trim_start()
        .starts_with('{')
}

fn parse_v2_root(text: &str) -> Result<Mapping, RuntimeEnvError> {
    let document = Document::from_str(text).map_err(|error| {
        RuntimeEnvError::new(format!("could not parse runtime manifest YAML: {error}"))
    })?;
    let root = document
        .as_mapping()
        .ok_or_else(|| RuntimeEnvError::new("runtime manifest root must be a mapping"))?;
    if root.get("version").and_then(|value| value.to_i64()) != Some(2) {
        return Err(RuntimeEnvError::new("NORMALIZE_MANIFEST_V2_REQUIRED: Compose normalization requires runtime manifest version 2"));
    }
    Ok(root)
}

fn missing_files(manifest: &RuntimeManifest, repo: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let existing = manifest
        .resources()
        .compose
        .files
        .iter()
        .collect::<HashSet<_>>();
    files
        .iter()
        .map(|file| file.strip_prefix(repo).unwrap_or(file).to_path_buf())
        .filter(|file| !existing.contains(file))
        .collect()
}

fn manifest_edits(
    source: &str,
    root: &Mapping,
    files: &[PathBuf],
    exports: &[&ComposeExport],
) -> Result<Vec<ByteEdit>, RuntimeEnvError> {
    let Some(resources) = root.get_mapping("resources") else {
        return Ok(vec![insert_at_mapping_end(
            source,
            root,
            format!("resources:\n{}", compose_block(2, files, exports)),
        )]);
    };
    let Some(compose) = resources.get_mapping("compose") else {
        return Ok(vec![insert_at_mapping_end(
            source,
            &resources,
            compose_block(0, files, exports),
        )]);
    };
    let mut edits = Vec::new();
    if !files.is_empty() {
        append_files(source, &compose, files, &mut edits);
    }
    if !exports.is_empty() {
        append_exports(source, &compose, exports, &mut edits);
    }
    Ok(edits)
}

#[derive(Clone)]
struct ByteEdit {
    start: usize,
    end: usize,
    replacement: String,
}

fn append_files(source: &str, compose: &Mapping, files: &[PathBuf], edits: &mut Vec<ByteEdit>) {
    if let Some(sequence) = compose.get_sequence("files") {
        let values = files
            .iter()
            .map(|path| yaml_scalar(&logical_path(path)))
            .collect::<Vec<_>>();
        edits.push(append_sequence(source, sequence.byte_range(), &values, 6));
    } else {
        let values = files
            .iter()
            .map(|path| format!("      - {}\n", yaml_scalar(&logical_path(path))))
            .collect::<String>();
        edits.push(insert_at_mapping_end(
            source,
            compose,
            format!("files:\n{values}"),
        ));
    }
}

fn append_exports(
    source: &str,
    compose: &Mapping,
    exports: &[&ComposeExport],
    edits: &mut Vec<ByteEdit>,
) {
    if let Some(sequence) = compose.get_sequence("exports") {
        let range = sequence.byte_range();
        let slice = &source[range.start as usize..range.end as usize];
        if slice.trim_start().starts_with('[') {
            let values = exports
                .iter()
                .map(|export| flow_export(export))
                .collect::<Vec<_>>();
            edits.push(append_sequence(source, range, &values, 6));
        } else {
            let blocks = exports
                .iter()
                .map(|export| export_block(export, 6))
                .collect::<String>();
            edits.push(append_block_sequence(source, range, blocks));
        }
    } else {
        let blocks = exports
            .iter()
            .map(|export| export_block(export, 6))
            .collect::<String>();
        edits.push(insert_at_mapping_end(
            source,
            compose,
            format!("exports:\n{blocks}"),
        ));
    }
}

fn compose_block(indent: usize, files: &[PathBuf], exports: &[&ComposeExport]) -> String {
    let pad = " ".repeat(indent);
    let mut body = format!("{pad}compose:\n");
    if !files.is_empty() {
        body.push_str(&format!("{pad}  files:\n"));
        for file in files {
            body.push_str(&format!(
                "{pad}    - {}\n",
                yaml_scalar(&logical_path(file))
            ));
        }
    }
    if !exports.is_empty() {
        body.push_str(&format!("{pad}  exports:\n"));
        for export in exports {
            body.push_str(&export_block(export, indent + 4));
        }
    }
    body
}

fn export_block(export: &ComposeExport, indent: usize) -> String {
    let pad = " ".repeat(indent);
    format!(
        "{pad}- service: {}\n{pad}  target: {}\n{pad}  protocol: {}\n{pad}  env: {}\n{pad}  value: {}\n",
        export.service,
        export.target,
        protocol_name(export),
        export.env,
        value_name(export)
    )
}

fn flow_export(export: &ComposeExport) -> String {
    format!(
        "{{service: {}, target: {}, protocol: {}, env: {}, value: {}}}",
        yaml_scalar(&export.service),
        export.target,
        protocol_name(export),
        yaml_scalar(&export.env),
        value_name(export)
    )
}

fn insert_at_mapping_end(source: &str, mapping: &Mapping, block: String) -> ByteEdit {
    let range = mapping.byte_range();
    let start = after_line(source, range.end as usize);
    let indent = line_indent(source, range.start as usize);
    let replacement = block
        .lines()
        .map(|line| format!("{}{line}\n", " ".repeat(indent)))
        .collect();
    ByteEdit {
        start,
        end: start,
        replacement,
    }
}

fn append_sequence(
    source: &str,
    range: yaml_edit::TextPosition,
    values: &[String],
    indent: usize,
) -> ByteEdit {
    let slice = &source[range.start as usize..range.end as usize];
    if slice.trim_start().starts_with('[') {
        let end = range.start as usize + slice.rfind(']').expect("flow sequence closes");
        let separator = if slice.trim_matches(&[' ', '[', ']'][..]).is_empty() {
            ""
        } else {
            ", "
        };
        return ByteEdit {
            start: end,
            end,
            replacement: format!("{separator}{}", values.join(", ")),
        };
    }
    let start = after_line(source, range.end as usize);
    ByteEdit {
        start,
        end: start,
        replacement: values
            .iter()
            .map(|value| format!("{}- {value}\n", " ".repeat(indent)))
            .collect(),
    }
}

fn append_block_sequence(
    source: &str,
    range: yaml_edit::TextPosition,
    replacement: String,
) -> ByteEdit {
    let start = after_line(source, range.end as usize);
    ByteEdit {
        start,
        end: start,
        replacement,
    }
}

fn after_line(source: &str, offset: usize) -> usize {
    source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index + 1)
}

fn line_indent(source: &str, offset: usize) -> usize {
    let start = source[..offset].rfind('\n').map_or(0, |index| index + 1);
    source[start..offset]
        .bytes()
        .take_while(|byte| *byte == b' ')
        .count()
}

fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization cannot fail")
}

fn apply_byte_edits(source: &str, mut edits: Vec<ByteEdit>) -> Vec<u8> {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
    let mut rendered = source.to_string();
    for edit in edits {
        rendered.replace_range(edit.start..edit.end, &edit.replacement);
    }
    rendered.into_bytes()
}

fn protocol_name(export: &ComposeExport) -> &'static str {
    match export.protocol {
        ExportProtocol::Http => "http",
        ExportProtocol::Https => "https",
        ExportProtocol::Tcp => "tcp",
        ExportProtocol::Udp => "udp",
    }
}

fn value_name(export: &ComposeExport) -> &'static str {
    match export.value {
        ExportValue::Url => "url",
        ExportValue::Port => "port",
        ExportValue::HostPort => "host-port",
    }
}
