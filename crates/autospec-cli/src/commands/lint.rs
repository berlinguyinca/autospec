use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use autospec_core::lint::{
    directive_for, lint_implementation, lint_issue_body, parse_unified_diff,
    ImplementationLintContext, ImplementationLintFinding, ImplementationLintOptions,
    ImplementationLintSeverity, IssueLintFinding, RepositoryIndex, UnifiedDiff,
};

use super::CommandFailure;

pub fn run(args: &[String]) -> Result<(), CommandFailure> {
    match args {
        [] => Err(CommandFailure::diagnostic(
            "autospec lint requires a subcommand",
        )),
        [flag] if flag == "--help" || flag == "-h" => {
            print_help();
            Ok(())
        }
        [command, rest @ ..] if command == "issue" => run_issue(rest),
        [command, rest @ ..] if command == "implementation" => run_implementation(rest),
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec lint command: {command}"
        ))),
    }
}

fn run_implementation(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_implementation_help();
        return Ok(());
    }

    let options = parse_implementation_options(args)?;
    let root = std::env::current_dir().map_err(|error| {
        implementation_failure(format!("could not determine the repository root: {error}"))
    })?;
    let Some(input) = read_implementation_input(&options, &root)? else {
        eprintln!("autospec lint implementation: no staged changes found");
        return Ok(());
    };
    let diff = parse_unified_diff(&input.diff).map_err(|error| {
        implementation_failure(format!("could not parse unified diff: {error}"))
    })?;
    let excluded_diff_source = in_repository_diff_source(&options.source, &root);
    let repository = FilesystemRepositoryIndex::from_diff(
        &root,
        &diff,
        std::env::var("AUTOSPEC_REUSE_LENS").is_ok_and(|value| value == "1"),
        excluded_diff_source.as_deref(),
    );
    let mut lint_options = ImplementationLintOptions {
        enable_vacuous_assertions: options.vacuous_assertions,
        enable_assertion_density: options.assertion_density,
        pre_commit_mode: options.pre_commit,
        enable_reuse_lens: std::env::var("AUTOSPEC_REUSE_LENS").is_ok_and(|value| value == "1"),
        ..ImplementationLintOptions::default()
    };
    lint_options.aggregate_hard_cap = test_hard_cap(lint_options.aggregate_hard_cap);
    let result = lint_implementation(
        &diff,
        ImplementationLintContext {
            issue_body: input.issue_body.as_deref(),
            repository: &repository,
            options: lint_options,
        },
    );

    if options.directives {
        print_implementation_directives(&result.findings);
    } else {
        print_implementation_findings(&result.findings);
    }

    let exit_code = result.exit_code();
    if exit_code == 0 {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), exit_code))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImplementationSource {
    PullRequest(String),
    DiffFile(PathBuf),
    Staged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImplementationOptions {
    source: ImplementationSource,
    issue: Option<String>,
    pre_commit: bool,
    directives: bool,
    vacuous_assertions: bool,
    assertion_density: bool,
}

struct ImplementationInput {
    diff: String,
    issue_body: Option<String>,
}

fn parse_implementation_options(args: &[String]) -> Result<ImplementationOptions, CommandFailure> {
    let mut pull_request = None;
    let mut diff_file = None;
    let mut staged = false;
    let mut pre_commit = false;
    let mut issue = None;
    let mut directives = false;
    let mut vacuous_assertions = false;
    let mut assertion_density = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--issue" => {
                let value = argument_value(args, &mut index, "--issue")?;
                set_once(
                    &mut issue,
                    value,
                    "--issue accepts exactly one issue number",
                )?;
            }
            "--diff-file" => {
                let value = argument_value(args, &mut index, "--diff-file")?;
                set_once(
                    &mut diff_file,
                    PathBuf::from(value),
                    "--diff-file accepts exactly one path",
                )?;
            }
            "--pre-commit" => {
                pre_commit = true;
                staged = true;
                vacuous_assertions = true;
                assertion_density = true;
            }
            "--staged" => staged = true,
            "--directives" => directives = true,
            "--vacuous-assertions" => vacuous_assertions = true,
            "--assertion-density" => assertion_density = true,
            "--help" | "-h" => {
                return Err(implementation_diagnostic(
                    "--help cannot be combined with implementation lint arguments",
                    2,
                ));
            }
            option if option.starts_with('-') => {
                return Err(implementation_diagnostic(
                    &format!("unknown option: {option}"),
                    2,
                ));
            }
            value => set_once(
                &mut pull_request,
                value.to_owned(),
                "accepts exactly one <PR>",
            )?,
        }
        index += 1;
    }

    let source_count = usize::from(pull_request.is_some())
        + usize::from(diff_file.is_some())
        + usize::from(staged);
    if source_count == 0 {
        return Err(implementation_diagnostic(
            "must supply <PR>, --diff-file <path>, or --staged",
            2,
        ));
    }
    if source_count > 1 {
        let message = if pull_request.is_some() && diff_file.is_some() {
            "--diff-file and <PR> are mutually exclusive"
        } else {
            "<PR>, --diff-file, and --staged are mutually exclusive input sources"
        };
        return Err(implementation_diagnostic(message, 2));
    }

    let source = match (pull_request, diff_file, staged) {
        (Some(pull_request), None, false) => ImplementationSource::PullRequest(pull_request),
        (None, Some(diff_file), false) => ImplementationSource::DiffFile(diff_file),
        (None, None, true) => ImplementationSource::Staged,
        _ => unreachable!("implementation source count was validated"),
    };
    Ok(ImplementationOptions {
        source,
        issue,
        pre_commit,
        directives,
        vacuous_assertions,
        assertion_density,
    })
}

fn argument_value(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<String, CommandFailure> {
    *index += 1;
    let Some(value) = args.get(*index) else {
        return Err(implementation_diagnostic(
            &format!("{option} requires an argument"),
            2,
        ));
    };
    if value.starts_with('-') || value.is_empty() {
        return Err(implementation_diagnostic(
            &format!("{option} requires an argument"),
            2,
        ));
    }
    Ok(value.to_owned())
}

fn set_once<T>(slot: &mut Option<T>, value: T, message: &str) -> Result<(), CommandFailure> {
    if slot.replace(value).is_some() {
        return Err(implementation_diagnostic(message, 2));
    }
    Ok(())
}

fn read_implementation_input(
    options: &ImplementationOptions,
    root: &Path,
) -> Result<Option<ImplementationInput>, CommandFailure> {
    match &options.source {
        ImplementationSource::DiffFile(path) => {
            let diff = fs::read_to_string(path).map_err(|error| {
                implementation_failure(format!(
                    "could not read diff file {}: {error}",
                    path.display()
                ))
            })?;
            Ok(Some(ImplementationInput {
                diff,
                issue_body: None,
            }))
        }
        ImplementationSource::Staged => {
            let diff = run_checked_command("git", ["diff", "--cached"], root)?;
            if diff.is_empty() {
                Ok(None)
            } else {
                Ok(Some(ImplementationInput {
                    diff,
                    issue_body: None,
                }))
            }
        }
        ImplementationSource::PullRequest(pull_request) => {
            let diff = run_checked_command("gh", ["pr", "diff", pull_request], root)?;
            let issue_body = options
                .issue
                .as_deref()
                .map(|issue| {
                    run_checked_command(
                        "gh",
                        ["issue", "view", issue, "--json", "body", "--jq", ".body"],
                        root,
                    )
                })
                .transpose()?;
            Ok(Some(ImplementationInput { diff, issue_body }))
        }
    }
}

fn run_checked_command<const N: usize>(
    program: &str,
    arguments: [&str; N],
    root: &Path,
) -> Result<String, CommandFailure> {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(root)
        .output()
        .map_err(|error| {
            implementation_failure(format!(
                "could not run {program} {}: {error}",
                arguments.join(" ")
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr
        };
        return Err(implementation_failure(format!(
            "{program} {} failed: {detail}",
            arguments.join(" ")
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        implementation_failure(format!(
            "{program} {} returned non-UTF-8 output: {error}",
            arguments.join(" ")
        ))
    })
}

fn print_implementation_findings(findings: &[ImplementationLintFinding]) {
    for finding in findings {
        let severity = match finding.severity {
            ImplementationLintSeverity::Error => "",
            ImplementationLintSeverity::Info => "INFO:",
        };
        let line = finding
            .line
            .map(|line| line.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{severity}{}:{}:{line}: {}",
            finding.rule_id(),
            finding.path,
            finding.message
        );
    }
}

fn print_implementation_directives(findings: &[ImplementationLintFinding]) {
    for finding in findings
        .iter()
        .filter(|finding| finding.severity == ImplementationLintSeverity::Error)
    {
        println!("Fix {}: {}", finding.rule_id(), directive_for(finding.rule));
    }
}

fn implementation_failure(message: String) -> CommandFailure {
    implementation_diagnostic(&message, 1)
}

fn implementation_diagnostic(message: &str, exit_code: i32) -> CommandFailure {
    CommandFailure::status(
        format!("autospec lint implementation: {message}"),
        exit_code,
    )
}

/// A test-only switch lets the public CLI prove the terminal status without
/// weakening the production 200-finding cap. Release builds always use the
/// core default; the clearly named override is available to debug test binaries.
fn test_hard_cap(default: usize) -> usize {
    if cfg!(debug_assertions) {
        return std::env::var("AUTOSPEC_LINT_IMPLEMENTATION_TEST_HARD_CAP")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(default);
    }
    default
}

fn in_repository_diff_source(source: &ImplementationSource, root: &Path) -> Option<String> {
    let ImplementationSource::DiffFile(path) = source else {
        return None;
    };
    let root = root.canonicalize().ok()?;
    let diff = path.canonicalize().ok()?;
    diff.strip_prefix(root)
        .ok()?
        .to_str()
        .map(|path| path.replace('\\', "/"))
}

#[derive(Default)]
struct FilesystemRepositoryIndex {
    helper_definitions: BTreeMap<String, String>,
    external_callers: BTreeMap<String, usize>,
    post_change_files: BTreeMap<String, String>,
}

impl FilesystemRepositoryIndex {
    fn from_diff(
        root: &Path,
        diff: &UnifiedDiff,
        include_reuse_lens: bool,
        excluded_diff_source: Option<&str>,
    ) -> Self {
        let mut index = Self::default();
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        for file in &diff.files {
            let Some(path) = contained_snapshot_path(&canonical_root, &file.path) else {
                continue;
            };
            if let Ok(contents) = fs::read_to_string(path) {
                index.post_change_files.insert(file.path.clone(), contents);
            }
        }
        if include_reuse_lens {
            index.populate_reuse_evidence(&canonical_root, diff, excluded_diff_source);
        }
        index
    }

    fn populate_reuse_evidence(
        &mut self,
        root: &Path,
        diff: &UnifiedDiff,
        excluded_diff_source: Option<&str>,
    ) {
        if let Some(script_paths) = ripgrep_script_paths(root) {
            for relative in script_paths
                .into_iter()
                .filter(|path| Some(path.as_str()) != excluded_diff_source)
            {
                let Ok(contents) = fs::read_to_string(root.join(&relative)) else {
                    continue;
                };
                for name in shell_function_names(&contents) {
                    self.helper_definitions
                        .entry(name)
                        .or_insert_with(|| relative.clone());
                }
            }
        }
        for file in &diff.files {
            let Some(stem) = abstraction_stem(&file.path) else {
                continue;
            };
            if let Some(paths) = ripgrep_matching_paths(root, &stem) {
                let callers = paths
                    .iter()
                    .filter(|path| {
                        path.as_str() != file.path && Some(path.as_str()) != excluded_diff_source
                    })
                    .count();
                self.external_callers.insert(stem, callers);
            }
        }
    }
}

impl RepositoryIndex for FilesystemRepositoryIndex {
    fn helper_definition(&self, function_name: &str, excluding_path: &str) -> Option<String> {
        self.helper_definitions
            .get(function_name)
            .filter(|path| path.as_str() != excluding_path)
            .cloned()
    }

    fn external_caller_count(&self, stem: &str, _excluding_path: &str) -> Option<usize> {
        self.external_callers.get(stem).copied()
    }

    fn post_change_file(&self, path: &str) -> Option<String> {
        self.post_change_files.get(path).cloned()
    }
}

fn contained_snapshot_path(root: &Path, value: &str) -> Option<PathBuf> {
    let relative = safe_relative_path(value)?;
    let resolved = root.join(relative).canonicalize().ok()?;
    resolved.starts_with(root).then_some(resolved)
}

fn safe_relative_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn ripgrep_script_paths(root: &Path) -> Option<Vec<String>> {
    ripgrep_paths(
        root,
        ["--files", "scripts", "--glob", "*.sh", "--glob", "*.bash"],
    )
}

fn ripgrep_matching_paths(root: &Path, term: &str) -> Option<Vec<String>> {
    ripgrep_paths(root, ["--fixed-strings", term, "-l", "."])
}

fn ripgrep_paths<const N: usize>(root: &Path, arguments: [&str; N]) -> Option<Vec<String>> {
    let output = Command::new("rg")
        .args(arguments)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() && output.status.code() != Some(1) {
        return None;
    }
    String::from_utf8(output.stdout).ok().map(|paths| {
        paths
            .lines()
            .map(|path| path.trim_start_matches("./").replace('\\', "/"))
            .filter(|path| !path.is_empty())
            .collect()
    })
}

fn shell_function_names(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (line, requires_parentheses) = line
                .strip_prefix("function ")
                .map(|line| (line, false))
                .unwrap_or((line, true));
            let name = line
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect::<String>();
            if name.is_empty() {
                return None;
            }
            let rest = &line[name.len()..];
            if (!requires_parentheses || rest.trim_start().starts_with('{'))
                || rest.trim_start().starts_with("()")
            {
                Some(name)
            } else {
                None
            }
        })
        .collect()
}

fn abstraction_stem(path: &str) -> Option<String> {
    let filename = Path::new(path).file_stem()?.to_str()?;
    let lower = filename.to_ascii_lowercase();
    if [
        "manager", "factory", "adapter", "wrapper", "base", "abstract",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        Some(filename.to_owned())
    } else {
        None
    }
}

fn run_issue(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_issue_help();
        return Ok(());
    }
    let options = parse_issue_options(args)?;
    let body = read_body(&options.body_path)?;
    let findings = lint_issue_body(&body);

    if options.json {
        print_json(&findings);
    } else {
        print_text(&findings);
    }

    if findings.is_empty() {
        Ok(())
    } else {
        Err(CommandFailure::status(
            String::new(),
            findings.len().min(64) as i32,
        ))
    }
}

struct IssueOptions {
    body_path: String,
    json: bool,
}

fn parse_issue_options(args: &[String]) -> Result<IssueOptions, CommandFailure> {
    let mut body_path = None;
    let mut json = false;
    for argument in args {
        match argument.as_str() {
            "--json" => json = true,
            "--help" | "-h" => {
                return Err(CommandFailure::diagnostic(
                    "autospec lint issue --help cannot be combined with other arguments",
                ));
            }
            "-" => set_body_path(&mut body_path, argument)?,
            option if option.starts_with('-') => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec lint issue option: {option}"
                )));
            }
            path => set_body_path(&mut body_path, path)?,
        }
    }

    let Some(body_path) = body_path else {
        return Err(CommandFailure::diagnostic(
            "autospec lint issue requires a body path",
        ));
    };
    Ok(IssueOptions { body_path, json })
}

fn set_body_path(slot: &mut Option<String>, path: &str) -> Result<(), CommandFailure> {
    if slot.replace(path.to_owned()).is_some() {
        return Err(CommandFailure::diagnostic(
            "autospec lint issue accepts exactly one body path",
        ));
    }
    Ok(())
}

fn read_body(path: &str) -> Result<String, CommandFailure> {
    if path == "-" {
        let mut body = String::new();
        io::stdin().read_to_string(&mut body).map_err(|error| {
            CommandFailure::diagnostic(format!("could not read issue body from stdin: {error}"))
        })?;
        return Ok(body);
    }
    fs::read_to_string(path).map_err(|error| {
        CommandFailure::diagnostic(format!("could not read issue body {path}: {error}"))
    })
}

fn print_text(findings: &[IssueLintFinding]) {
    for finding in findings {
        eprintln!("{}: {}", finding.rule_id(), finding.message);
    }
}

fn print_json(findings: &[IssueLintFinding]) {
    if findings.is_empty() {
        println!("[]");
        return;
    }
    println!("[");
    for (index, finding) in findings.iter().enumerate() {
        let separator = if index + 1 == findings.len() { "" } else { "," };
        println!(
            "  {{\"rule\":\"{}\",\"description\":\"{}\"}}{separator}",
            finding.rule_id(),
            escape_json(&finding.message)
        );
    }
    println!("]");
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn print_help() {
    println!(
        "autospec lint\n\nUSAGE:\n    autospec lint <COMMAND>\n\nCOMMANDS:\n    issue            Lint an issue body\n    implementation   Lint an implementation diff"
    );
}

fn print_issue_help() {
    println!(
        "autospec lint issue\n\nUSAGE:\n    autospec lint issue [--json] <BODY_PATH>\n\nBODY_PATH:\n    -           Read the issue body from standard input\n\nOPTIONS:\n    --json      Write ordered findings as JSON\n    -h, --help  Print help"
    );
}

fn print_implementation_help() {
    println!(
        "autospec lint implementation\n\nUSAGE:\n    autospec lint implementation <PR> [--issue <N>] [OPTIONS]\n    autospec lint implementation --diff-file <PATH> [OPTIONS]\n    autospec lint implementation --pre-commit --staged [OPTIONS]\n\nINPUTS:\n    <PR>                         Read a pull-request diff with `gh pr diff`\n    --diff-file <PATH>           Read an offline unified-diff file\n    --staged                      Read `git diff --cached`\n\nOPTIONS:\n    --issue <N>                  Read Guardian skip directives for a remote PR only\n    --pre-commit                 Enable vacuous and assertion-density checks; implies --staged\n    --directives                 Render `Fix RULE_ID: ...` records\n    --vacuous-assertions         Enable VACUOUS_* checks\n    --assertion-density          Enable ASSERTION_DENSITY checks\n    -h, --help                   Print help\n\nEXIT STATUS:\n    0                            No blocking findings\n    1                            Input or remote-command failure\n    1..64                        Blocking-finding count, capped at 64\n    200                          Scope explosion"
    );
}
