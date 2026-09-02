use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use autospec_core::claim::{
    evaluate_claim_safety_with_trusted_actors, lint_issue_intent_with_trusted_actors,
    review_issue_safety_with_trusted_actors, ClaimSafetyDecision, ClaimSafetyInput,
    IssueIntentFinding, SafetyReviewVerdict,
};
use autospec_core::lint::{
    directive_for, lint_implementation, lint_issue_body, lint_issue_implementation_contract,
    parse_unified_diff, ImplementationLintContext, ImplementationLintFinding,
    ImplementationLintOptions, ImplementationLintSeverity, IssueLintFinding, RepositoryIndex,
    UnifiedDiff,
};
use yaml_edit::Document;

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
        [command, rest @ ..] if command == "implementation-contract" => {
            run_implementation_contract(rest)
        }
        [command, ..] => Err(CommandFailure::diagnostic(format!(
            "unknown autospec lint command: {command}"
        ))),
    }
}

fn run_implementation_contract(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_implementation_contract_help();
        return Ok(());
    }
    let (issue_body_path, diff_path) = parse_implementation_contract_options(args)?;
    let issue_body = read_implementation_contract_file(&issue_body_path, "issue body")?;
    let diff_source = read_implementation_contract_file(&diff_path, "diff")?;
    let diff = parse_unified_diff(&diff_source).map_err(|error| {
        implementation_contract_failure(format!("could not parse unified diff: {error}"))
    })?;
    let result = lint_issue_implementation_contract(&diff, &issue_body);
    print_implementation_findings(&result.findings);

    let exit_code = result.exit_code();
    if exit_code == 0 {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), exit_code))
    }
}

fn parse_implementation_contract_options(
    args: &[String],
) -> Result<(PathBuf, PathBuf), CommandFailure> {
    if args.len() != 4 {
        return Err(implementation_contract_diagnostic(
            "requires --issue-body-file <PATH> and --diff-file <PATH>",
            2,
        ));
    }
    let mut issue_body_file = None;
    let mut diff_file = None;
    for pair in args.chunks_exact(2) {
        let option = pair[0].as_str();
        let slot = match option {
            "--issue-body-file" => &mut issue_body_file,
            "--diff-file" => &mut diff_file,
            _ => {
                return Err(implementation_contract_diagnostic(
                    &format!("unknown option: {option}"),
                    2,
                ));
            }
        };
        if pair[1].starts_with('-') {
            return Err(implementation_contract_diagnostic(
                &format!("{option} requires an argument"),
                2,
            ));
        }
        if slot.replace(PathBuf::from(&pair[1])).is_some() {
            return Err(implementation_contract_diagnostic(
                &format!("{option} accepts exactly one path"),
                2,
            ));
        }
    }
    match (issue_body_file, diff_file) {
        (Some(issue_body_file), Some(diff_file)) => Ok((issue_body_file, diff_file)),
        _ => Err(implementation_contract_diagnostic(
            "requires --issue-body-file <PATH> and --diff-file <PATH>",
            2,
        )),
    }
}

fn read_implementation_contract_file(
    path: &Path,
    description: &str,
) -> Result<String, CommandFailure> {
    fs::read_to_string(path).map_err(|error| {
        implementation_contract_failure(format!(
            "could not read {description} file {}: {error}",
            path.display()
        ))
    })
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

fn implementation_contract_failure(message: String) -> CommandFailure {
    implementation_contract_diagnostic(&message, 1)
}

fn implementation_contract_diagnostic(message: &str, exit_code: i32) -> CommandFailure {
    CommandFailure::status(
        format!("autospec lint implementation-contract: {message}"),
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

struct ShellFunctionForm {
    prefix: &'static str,
    accepted_rest_prefixes: &'static [&'static str],
}

const SHELL_FUNCTION_FORMS: &[ShellFunctionForm] = &[
    ShellFunctionForm {
        prefix: "function ",
        accepted_rest_prefixes: &[],
    },
    ShellFunctionForm {
        prefix: "",
        accepted_rest_prefixes: &["{", "()"],
    },
];

fn shell_function_names(contents: &str) -> Vec<String> {
    contents.lines().filter_map(shell_function_name).collect()
}

fn shell_function_name(line: &str) -> Option<String> {
    let line = line.trim_start();
    SHELL_FUNCTION_FORMS.iter().find_map(|form| {
        line.strip_prefix(form.prefix).and_then(|line| {
            let line = line.trim_start();
            let name = line
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect::<String>();
            if name.is_empty() {
                return None;
            }
            let rest = &line[name.len()..];
            let rest = rest.trim_start();
            if form.accepted_rest_prefixes.is_empty()
                || form
                    .accepted_rest_prefixes
                    .iter()
                    .any(|prefix| rest.starts_with(prefix))
            {
                Some(name)
            } else {
                None
            }
        })
    })
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
    if let [command, rest @ ..] = args {
        if command == "safety" {
            return run_issue_safety(rest);
        }
    }
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

fn run_issue_safety(args: &[String]) -> Result<(), CommandFailure> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        print_issue_safety_help();
        return Ok(());
    }
    let options = parse_issue_safety_options(args)?;
    let body = read_body(&options.body_path)?;
    let policy = load_issue_safety_policy(options.config_path.as_deref())?;
    let trusted_actors = policy
        .trusted_actors
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut lint = lint_issue_intent_with_trusted_actors(
        &options.title,
        &body,
        &options.actor,
        &trusted_actors,
    );
    if policy.has_unsupported_pattern {
        lint.findings.push(IssueIntentFinding {
            severity: "block",
            rule_id: "invalid-policy-regex",
            pattern: "unsupported configured regex",
        });
        lint.blocking = true;
    }
    let decision = if lint.blocking {
        "SAFETY_BLOCK"
    } else if lint.ambiguous {
        "SAFETY_AMBIGUOUS"
    } else {
        "SAFETY_PASS"
    };

    if options.json {
        print_issue_safety_json(decision, &lint, &options.actor);
    } else {
        println!("{decision}");
        for finding in &lint.findings {
            println!(
                "RULE_ID: {}: {}: matched {}",
                finding.severity, finding.rule_id, finding.pattern
            );
        }
    }

    match decision {
        "SAFETY_PASS" => Ok(()),
        "SAFETY_AMBIGUOUS" => Err(CommandFailure::status(String::new(), 1)),
        _ => Err(CommandFailure::status(String::new(), 2)),
    }
}

struct IssueSafetyOptions {
    body_path: String,
    title: String,
    actor: String,
    config_path: Option<String>,
    json: bool,
}

fn parse_issue_safety_options(args: &[String]) -> Result<IssueSafetyOptions, CommandFailure> {
    let mut body_path = None;
    let mut title = None;
    let mut actor = None;
    let mut config_path = None;
    let mut json = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json = true,
            "--title" => set_once(
                &mut title,
                argument_value(args, &mut index, "--title")?,
                "--title accepts exactly one value",
            )?,
            "--actor" => set_once(
                &mut actor,
                argument_value(args, &mut index, "--actor")?,
                "--actor accepts exactly one value",
            )?,
            "--config" => set_once(
                &mut config_path,
                argument_value(args, &mut index, "--config")?,
                "--config accepts exactly one value",
            )?,
            "--help" | "-h" => {
                return Err(CommandFailure::diagnostic(
                    "autospec lint issue safety --help cannot be combined with other arguments",
                ));
            }
            "-" => set_body_path(&mut body_path, &args[index])?,
            option if option.starts_with('-') => {
                return Err(CommandFailure::diagnostic(format!(
                    "unknown autospec lint issue safety option: {option}"
                )));
            }
            path => set_body_path(&mut body_path, path)?,
        }
        index += 1;
    }
    let Some(body_path) = body_path else {
        return Err(CommandFailure::diagnostic(
            "autospec lint issue safety requires a body path",
        ));
    };
    Ok(IssueSafetyOptions {
        body_path,
        title: title.unwrap_or_default(),
        actor: actor.unwrap_or_default(),
        config_path,
        json,
    })
}

#[derive(Default)]
pub(crate) struct IssueSafetyPolicy {
    pub(crate) trusted_actors: Vec<String>,
    pub(crate) has_unsupported_pattern: bool,
}

pub(crate) fn load_issue_safety_policy(
    config_path: Option<&str>,
) -> Result<IssueSafetyPolicy, CommandFailure> {
    let (path, explicit) = match config_path {
        Some(path) => (PathBuf::from(path), true),
        None => match std::env::var_os("AUTOSPEC_CONFIG_FILE") {
            Some(path) => (PathBuf::from(path), true),
            None => (PathBuf::from(".autospec/autospec.yml"), false),
        },
    };
    let document = match fs::read_to_string(&path) {
        Ok(document) => document,
        Err(error) if !explicit && error.kind() == io::ErrorKind::NotFound => {
            return Ok(default_issue_safety_policy())
        }
        Err(error) => {
            return Err(CommandFailure::diagnostic(format!(
                "could not read issue safety policy {}: {error}",
                path.display()
            )))
        }
    };
    parse_issue_safety_policy(&document).ok_or_else(|| {
        CommandFailure::diagnostic(format!(
            "could not parse issue safety policy {}",
            path.display()
        ))
    })
}

pub(crate) fn review_issue_safety_for_queue(
    input: &ClaimSafetyInput,
) -> Result<SafetyReviewVerdict, CommandFailure> {
    let trusted_actors = configured_safety_trusted_actors()?;
    let trusted_actors = trusted_actors
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    Ok(review_issue_safety_with_trusted_actors(
        input,
        &trusted_actors,
    ))
}

pub(crate) fn confirm_issue_safety_for_queue(
    input: &ClaimSafetyInput,
) -> Result<bool, CommandFailure> {
    Ok(claim_safety_with_config(input)?.allowed)
}

pub(crate) fn claim_safety_with_config(
    input: &ClaimSafetyInput,
) -> Result<ClaimSafetyDecision, CommandFailure> {
    let trusted_actors = configured_safety_trusted_actors()?;
    let trusted_actors = trusted_actors
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    Ok(evaluate_claim_safety_with_trusted_actors(
        input,
        &trusted_actors,
    ))
}

pub(crate) fn configured_safety_trusted_actors() -> Result<Vec<String>, CommandFailure> {
    let policy = load_issue_safety_policy(None)?;
    if policy.has_unsupported_pattern {
        return Err(CommandFailure::diagnostic(
            "issue safety policy contains unsupported custom regex",
        ));
    }
    Ok(policy.trusted_actors)
}

fn default_issue_safety_policy() -> IssueSafetyPolicy {
    IssueSafetyPolicy {
        trusted_actors: vec!["berlinguyinca".to_string()],
        has_unsupported_pattern: false,
    }
}

fn parse_issue_safety_policy(document: &str) -> Option<IssueSafetyPolicy> {
    Document::from_str(document).ok()?.as_mapping()?;
    let mut policy = default_issue_safety_policy();
    let mut gate_indent = None;
    let mut section_indent = None;
    let mut pattern_indent = None;
    let mut trusted_indent = None;

    for raw_line in document.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = raw_line.len() - raw_line.trim_start().len();

        let Some(gate) = gate_indent else {
            if trimmed == "issue_intent_gate:" {
                gate_indent = Some(indent);
            }
            continue;
        };
        if indent <= gate {
            break;
        }

        if indent == gate + 2 {
            section_indent = None;
            pattern_indent = None;
            trusted_indent = None;
            match trimmed {
                "block_patterns:" | "ambiguous_patterns:" => section_indent = Some(indent),
                "trusted_actors:" => trusted_indent = Some(indent),
                _ => {}
            }
            continue;
        }

        if let Some(trusted) = trusted_indent {
            if indent > trusted && trimmed.starts_with("- login:") {
                let login = parse_yaml_scalar(trimmed.trim_start_matches("- login:"))?;
                if !login.is_empty() && !policy.trusted_actors.iter().any(|actor| actor == &login) {
                    policy.trusted_actors.push(login);
                }
            }
        }

        if let Some(section) = section_indent {
            if indent <= section {
                pattern_indent = None;
                continue;
            }
            if trimmed == "patterns:" {
                pattern_indent = Some(indent);
                continue;
            }
        }
        if let Some(patterns) = pattern_indent {
            if indent <= patterns {
                pattern_indent = None;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("- ") {
                let pattern = parse_yaml_scalar(value)?;
                if !is_builtin_issue_safety_pattern(&pattern) {
                    policy.has_unsupported_pattern = true;
                }
            }
        }
    }
    Some(policy)
}

fn parse_yaml_scalar(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with('[') || value.starts_with('{') {
        return None;
    }
    if let Some(value) = value.strip_prefix('"') {
        let value = value.strip_suffix('"')?;
        let mut result = String::new();
        let mut escaped = false;
        for character in value.chars() {
            if escaped {
                match character {
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    'n' => result.push('\n'),
                    'r' => result.push('\r'),
                    't' => result.push('\t'),
                    _ => return None,
                }
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                result.push(character);
            }
        }
        return (!escaped).then_some(result);
    }
    if let Some(value) = value.strip_prefix('\'') {
        return value.strip_suffix('\'').map(ToOwned::to_owned);
    }
    (!value.is_empty()).then(|| value.to_string())
}

fn is_builtin_issue_safety_pattern(pattern: &str) -> bool {
    matches!(
        pattern,
        "(?i)delete .*production"
            | "(?i)drop .*prod(uction)? .*database"
            | "(?i)(dump|print|exfiltrate|send).*secret"
            | "(?i)(aws|github|stripe).*token"
            // credential-printing. This allowlist is not a second evaluator:
            // `autospec_core::claim::contains_credential_printing` is the only
            // one. These strings declare what it implements — word boundaries,
            // line-scoped `[^\n]` instead of `.`, and an explicit verb→noun /
            // noun→destination order (issue #3113).
            | "(?i)\\b(print|prints|printed|printing|dump|dumps|dumped|dumping|log|logs|logged|logging|write|writes|show|shows|showed|showing|shown|expose|exposes|exposed|send|sends|sending)\\b[^\\n]*\\b(credential|credentials|password|passwords|api[ -]?keys?|private[ -]?keys?|token|tokens)\\b"
            | "(?i)\\b(credential|credentials|password|passwords|api[ -]?keys?|private[ -]?keys?|token|tokens)\\b[^\\n]*\\b(to (the )?(logs?|console|stdout)|in logs?)\\b"
            // Historical unbounded spellings, kept accepted so an existing
            // repo policy naming them is not flipped to `invalid-policy-regex`.
            | "(?i)(print|dump|log|write|show|expose|send).*(credential|password|api[ -]?key|private[ -]?key|token)"
            | "(?i)(credential|password|api[ -]?key|private[ -]?key|token).*(to (the )?(log|console|stdout)|in logs?)"
            | "(?i)ignore (all )?(previous|system|developer|agent) instructions"
            | "(?i)bypass (ci|tests|hooks|review|guardian)"
            | "(?i)(disable|turn off|remove|skip)[^.!?\\n]{0,120}(ci|checks?|tests?|hooks?|review|guardian)"
            | "(?i)(ci|checks?|tests?|hooks?|review|guardian)[^.!?\\n]{0,120}\\b(disabled?|turned? off|removed?|skip(?:ped)?|bypassed?|ignored?|suppressed?)\\b"
            | "(?i)\\b(auth|login|password|token|admin)[ -]?(backdoor|bypass)\\b"
            | "(?i)\\b(backdoor|bypass)\\b.*\\b(auth|login|password|token|admin)\\b"
            | "(?i)\\bmagic (token|password|login)\\b"
            | "rm -rf /"
            | "(?i)curl .*\\| *(sh|bash)"
            | "(?i)clean (old|bad|stale)? ?data"
            | "(?i)(relax|disable|remove).*security"
            | "(?i)(relax|disable|remove).*(auth|audit|logging)"
            | "(?i)\\b(production|prod|billing|payments|migration|terraform|iam|kms)\\b"
            | "(?i)(production|prod|billing|payments|migration|terraform|iam|kms)"
    )
}

fn print_issue_safety_json(
    decision: &str,
    lint: &autospec_core::claim::IssueIntentLint,
    actor: &str,
) {
    let findings = lint
        .findings
        .iter()
        .map(|finding| {
            format!(
                "{{\"severity\":\"{}\",\"rule_id\":\"{}\",\"pattern\":\"{}\"}}",
                finding.severity,
                finding.rule_id,
                escape_json(finding.pattern),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"decision\":\"{decision}\",\"findings\":[{findings}],\"actor\":{},\"trusted\":{}}}",
        if actor.is_empty() {
            "null".to_string()
        } else {
            format!("\"{}\"", escape_json(actor))
        },
        if lint.trusted { "true" } else { "false" },
    );
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
        "autospec lint\n\nUSAGE:\n    autospec lint <COMMAND>\n\nCOMMANDS:\n    issue                    Lint an issue body\n    implementation           Lint an implementation diff\n    implementation-contract  Lint issue-defined scope and regression evidence"
    );
}

fn print_issue_help() {
    println!(
        "autospec lint issue\n\nUSAGE:\n    autospec lint issue [--json] <BODY_PATH>\n\nBODY_PATH:\n    -           Read the issue body from standard input\n\nOPTIONS:\n    --json      Write ordered findings as JSON\n    -h, --help  Print help"
    );
}

fn print_issue_safety_help() {
    println!(
        "autospec lint issue safety\n\nUSAGE:\n    autospec lint issue safety [--json] [--actor LOGIN] [--title TITLE] [--config PATH] <BODY_PATH>\n\nBODY_PATH:\n    -           Read the issue body from standard input\n\nOPTIONS:\n    --json      Write the safety decision and findings as JSON\n    --actor     Identify the issue author for trusted-reset policy\n    --title     Include the issue title in policy evaluation\n    --config    Load trusted actors; unsupported custom regexes fail closed\n    -h, --help  Print help"
    );
}

fn print_implementation_help() {
    println!(
        "autospec lint implementation\n\nUSAGE:\n    autospec lint implementation <PR> [--issue <N>] [OPTIONS]\n    autospec lint implementation --diff-file <PATH> [OPTIONS]\n    autospec lint implementation --pre-commit --staged [OPTIONS]\n\nINPUTS:\n    <PR>                         Read a pull-request diff with `gh pr diff`\n    --diff-file <PATH>           Read an offline unified-diff file\n    --staged                      Read `git diff --cached`\n\nOPTIONS:\n    --issue <N>                  Read Guardian skip directives for a remote PR only\n    --pre-commit                 Enable vacuous and assertion-density checks; implies --staged\n    --directives                 Render `Fix RULE_ID: ...` records\n    --vacuous-assertions         Enable VACUOUS_* checks\n    --assertion-density          Enable ASSERTION_DENSITY checks\n    -h, --help                   Print help\n\nEXIT STATUS:\n    0                            No blocking findings\n    1                            Input or remote-command failure\n    1..64                        Blocking-finding count, capped at 64\n    200                          Scope explosion"
    );
}

fn print_implementation_contract_help() {
    println!(
        "autospec lint implementation-contract\n\nUSAGE:\n    autospec lint implementation-contract --issue-body-file <PATH> --diff-file <PATH>\n\nINPUTS:\n    --issue-body-file <PATH>  Read issue policy from a literal body file\n    --diff-file <PATH>        Read an offline unified-diff file\n\nOPTIONS:\n    -h, --help                Print help\n\nEXIT STATUS:\n    0                         No blocking findings\n    1                         Input failure or one blocking finding\n    2                         Option/usage error or two blocking findings\n    3..64                     Blocking-finding count, capped at 64\n    200                       Scope explosion"
    );
}

#[cfg(test)]
mod tests {
    use super::{is_builtin_issue_safety_pattern, shell_function_names};

    /// Issue #3113 AC #4. `is_builtin_issue_safety_pattern` is an allowlist for
    /// repo policy YAML, not a second evaluator — the only evaluator is
    /// `autospec_core::claim::contains_credential_printing`. Pin the declared
    /// credential-printing strings so they cannot silently re-diverge from the
    /// narrowed core semantics: word boundaries, line-scoped `[^\n]` rather
    /// than `.`, and an explicit order.
    #[test]
    fn credential_printing_patterns_declare_the_narrowed_core_semantics() {
        let narrowed = [
            "(?i)\\b(print|prints|printed|printing|dump|dumps|dumped|dumping|log|logs|logged|logging|write|writes|show|shows|showed|showing|shown|expose|exposes|exposed|send|sends|sending)\\b[^\\n]*\\b(credential|credentials|password|passwords|api[ -]?keys?|private[ -]?keys?|token|tokens)\\b",
            "(?i)\\b(credential|credentials|password|passwords|api[ -]?keys?|private[ -]?keys?|token|tokens)\\b[^\\n]*\\b(to (the )?(logs?|console|stdout)|in logs?)\\b",
        ];

        for pattern in narrowed {
            assert!(
                is_builtin_issue_safety_pattern(pattern),
                "narrowed pattern is no longer declared: {pattern}"
            );
            assert!(pattern.contains("\\b"), "missing word boundary: {pattern}");
            assert!(!pattern.contains(".*"), "unbounded gap survives: {pattern}");
        }

        // The historical spellings stay accepted so an existing repo policy
        // naming them is not flipped to `invalid-policy-regex`.
        assert!(is_builtin_issue_safety_pattern(
            "(?i)(print|dump|log|write|show|expose|send).*(credential|password|api[ -]?key|private[ -]?key|token)"
        ));
    }

    #[test]
    fn shell_function_names_preserves_current_shell_declaration_forms() {
        let script = r#"
            function legacy_style {
              :
            }
            function compact_style{
              :
            }
            modern_style() {
              :
            }
            spaced_modern_style () {
              :
            }
            bare_brace_style {
              :
            }
            bare_name_style {
              :
            }
        "#;

        assert_eq!(
            shell_function_names(script),
            [
                "legacy_style",
                "compact_style",
                "modern_style",
                "spaced_modern_style",
                "bare_brace_style",
                "bare_name_style",
            ]
        );
    }
}
