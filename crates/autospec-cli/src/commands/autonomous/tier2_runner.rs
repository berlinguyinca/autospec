use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use autospec_core::autonomous::tier2::{
    evaluate_tier2, tier2_verifier_candidate_keys, Tier2Complexity, Tier2Failure, Tier2FailureCode,
    Tier2GeneratedProposals, Tier2Input, Tier2Observation, Tier2Proposal, Tier2RoiPolicy,
    Tier2Severity, Tier2Source, Tier2Stage, Tier2StageResult, Tier2Verification,
    Tier2VerifierVerdicts, TIER2_RANK_LIMIT,
};
use autospec_core::explore::specialists::{
    collect_strict_domains, FileLineEvidence, StrictCollectorError, StrictCollectorErrorCode,
    StrictCollectorEvidence, StrictCollectorOptions,
};
#[cfg(unix)]
use nix::sys::signal::{killpg, Signal};
#[cfg(unix)]
use nix::unistd::Pid;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MODEL_TIMEOUT: Duration = Duration::from_secs(120);
const MODEL_OUTPUT_LIMIT: u64 = 256 * 1024;
const PROMPT_EVIDENCE_LIMIT: usize = 80;
const PROPOSAL_EVIDENCE_LIMIT: usize = 8;
static MODEL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[allow(clippy::large_enum_variant)] // Preserves the plan's explicit typed injection seam.
pub(super) enum Tier2Scan {
    NotRun,
    Complete(Tier2Observation),
    Failed(Tier2Failure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelStage {
    Generator,
    Verifier,
}

#[derive(Debug, Clone)]
pub(super) struct HarnessInvocation {
    pub(super) program: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) current_dir: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum HarnessKind {
    Codex,
    Claude,
    OpenCode,
    Pi,
}

pub(super) fn scan_native(repo_dir: &Path) -> Tier2Scan {
    let collector = match collect_strict_domains(&StrictCollectorOptions::new(repo_dir)) {
        Ok(collector) => collector,
        Err(error) => return Tier2Scan::Failed(strict_collector_failure(error)),
    };
    let harness = match resolve_harness() {
        Ok(harness) => harness,
        Err(detail) => {
            return generator_failure(collector, Tier2FailureCode::InvalidProposal, detail)
        }
    };
    scan_collected_with(collector, |stage, prompt| {
        let scratch = create_model_scratch()?;
        let result = harness
            .invocation(&scratch, prompt)
            .and_then(|invocation| run_bounded_child(&invocation, MODEL_TIMEOUT))
            .map_err(|detail| format!("{} {}", stage.as_str(), detail));
        let _ = fs::remove_dir_all(scratch);
        result
    })
}

pub(super) fn scan_collected_with(
    collector: StrictCollectorEvidence,
    mut invoke: impl FnMut(ModelStage, &str) -> Result<String, String>,
) -> Tier2Scan {
    let generator_prompt = generator_prompt(&collector);
    let generated_raw = match invoke(ModelStage::Generator, &generator_prompt) {
        Ok(raw) => raw,
        Err(detail) => {
            return generator_failure(collector, Tier2FailureCode::InvalidProposal, detail)
        }
    };
    let generated = match parse_generated(&generated_raw) {
        Ok(generated) => generated,
        Err(detail) => {
            return generator_failure(collector, Tier2FailureCode::InvalidProposal, detail)
        }
    };
    let verifier_keys = match tier2_verifier_candidate_keys(&generated.proposals) {
        Ok(keys) => keys,
        Err(_) => return evaluate_scan(collector, generated, Tier2StageResult::Missing),
    };
    let verifier_prompt = verifier_prompt(&collector, &generated, &verifier_keys);
    let verifier_raw = match invoke(ModelStage::Verifier, &verifier_prompt) {
        Ok(raw) => raw,
        Err(detail) => return verifier_failure(collector, generated, detail),
    };
    let verifier = match parse_verifier(&verifier_raw) {
        Ok(verifier) => verifier,
        Err(detail) => return verifier_failure(collector, generated, detail),
    };
    evaluate_scan(collector, generated, Tier2StageResult::Complete(verifier))
}

impl ModelStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Generator => "generator",
            Self::Verifier => "verifier",
        }
    }
}

impl HarnessKind {
    pub(super) fn invocation(
        self,
        scratch_dir: &Path,
        prompt: &str,
    ) -> Result<HarnessInvocation, String> {
        Ok(match self {
            Self::Codex => HarnessInvocation {
                program: "codex".into(),
                args: vec![
                    "exec".to_string(),
                    "--sandbox".to_string(),
                    "read-only".to_string(),
                    "--ephemeral".to_string(),
                    "--skip-git-repo-check".to_string(),
                    "--cd".to_string(),
                    scratch_dir.display().to_string(),
                    prompt.to_string(),
                ],
                current_dir: scratch_dir.to_path_buf(),
            },
            Self::Claude => HarnessInvocation {
                program: "claude".into(),
                args: vec![
                    "-p".to_string(),
                    "--safe-mode".to_string(),
                    "--permission-mode".to_string(),
                    "plan".to_string(),
                    "--tools".to_string(),
                    String::new(),
                    "--no-session-persistence".to_string(),
                    prompt.to_string(),
                ],
                current_dir: scratch_dir.to_path_buf(),
            },
            Self::OpenCode => {
                return Err(
                    "OpenCode native Tier 2 is disabled: no proven no-tools mode is available"
                        .to_string(),
                )
            }
            Self::Pi => HarnessInvocation {
                program: "pi".into(),
                args: vec![
                    "--provider".to_string(),
                    std::env::var("PI_PROVIDER").unwrap_or_else(|_| "default".to_string()),
                    prompt.to_string(),
                ],
                current_dir: scratch_dir.to_path_buf(),
            },
        })
    }
}

pub(super) fn create_model_scratch() -> Result<PathBuf, String> {
    let sequence = MODEL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "autospec-tier2-scratch-{}-{sequence}",
        std::process::id()
    ));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    builder.mode(0o700);
    builder
        .create(&path)
        .map_err(|error| format!("cannot create model scratch: {error}"))?;
    Ok(path)
}

fn resolve_harness() -> Result<HarnessKind, String> {
    if let Ok(explicit) = std::env::var("AUTOSPEC_HANDOFF_DISPATCHER_KIND") {
        return parse_harness(&explicit);
    }
    if std::env::var_os("CODEX_THREAD_ID").is_some() || std::env::var_os("CODEX_CI").is_some() {
        return Ok(HarnessKind::Codex);
    }
    if std::env::var_os("CLAUDECODE").is_some()
        || std::env::var_os("CLAUDE_CODE_ENTRYPOINT").is_some()
    {
        return Ok(HarnessKind::Claude);
    }
    if std::env::var_os("OPENCODE").is_some() || std::env::var_os("OPENCODE_SESSION_ID").is_some() {
        return Ok(HarnessKind::OpenCode);
    }
    if std::env::var_os("PI_CODING_AGENT").is_some() || std::env::var_os("PI_SESSION_ID").is_some()
    {
        return Ok(HarnessKind::Pi);
    }
    for (binary, kind) in [
        ("codex", HarnessKind::Codex),
        ("claude", HarnessKind::Claude),
        ("opencode", HarnessKind::OpenCode),
        ("pi", HarnessKind::Pi),
    ] {
        if binary_on_path(binary) {
            return Ok(kind);
        }
    }
    Err("no active Codex, Claude, OpenCode, or Pi harness is available".to_string())
}

fn parse_harness(value: &str) -> Result<HarnessKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "codex" => Ok(HarnessKind::Codex),
        "claude" | "claude-code" => Ok(HarnessKind::Claude),
        "opencode" => Ok(HarnessKind::OpenCode),
        "pi" => Ok(HarnessKind::Pi),
        other => Err(format!("unsupported autonomous harness {other}")),
    }
}

fn binary_on_path(binary: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(binary).is_file())
    })
}

pub(super) fn run_bounded_child(
    invocation: &HarnessInvocation,
    timeout: Duration,
) -> Result<String, String> {
    let sequence = MODEL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let output_path = std::env::temp_dir().join(format!(
        "autospec-tier2-model-{}-{sequence}.out",
        std::process::id()
    ));
    let mut output_guard = OutputGuard::create(output_path)?;
    let mut command = Command::new(&invocation.program);
    command
        .args(&invocation.args)
        .current_dir(&invocation.current_dir)
        .stdout(Stdio::from(output_guard.take_file()?))
        .stderr(Stdio::null());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Err(format!(
                "cannot spawn {}: {error}",
                invocation.program.display()
            ))
        }
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => match output_guard.len() {
                Ok(size) if size > MODEL_OUTPUT_LIMIT => {
                    terminate_child_group(&mut child);
                    break Err("child output exceeds 256 KiB".to_string());
                }
                Err(error) => {
                    terminate_child_group(&mut child);
                    break Err(format!("cannot inspect child output: {error}"));
                }
                _ if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    terminate_child_group(&mut child);
                    break Err("child timeout".to_string());
                }
            },
            Err(error) => {
                terminate_child_group(&mut child);
                break Err(format!("cannot wait for child: {error}"));
            }
        }
    };
    let result = match status {
        Err(detail) => Err(detail),
        Ok(status) if !status.success() => {
            Err(format!("child exited {}", status.code().unwrap_or(1)))
        }
        Ok(_) => {
            if output_guard
                .len()
                .map_err(|error| format!("cannot inspect child output: {error}"))?
                > MODEL_OUTPUT_LIMIT
            {
                Err("child output exceeds 256 KiB".to_string())
            } else {
                fs::read_to_string(&output_guard.path)
                    .map_err(|error| format!("cannot read child output: {error}"))
            }
        }
    };
    result
}

struct OutputGuard {
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl OutputGuard {
    fn create(path: PathBuf) -> Result<Self, String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&path)
            .map_err(|error| format!("cannot create bounded output: {error}"))?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }

    fn take_file(&mut self) -> Result<std::fs::File, String> {
        self.file
            .take()
            .ok_or_else(|| "bounded output file was already consumed".to_string())
    }

    fn len(&self) -> std::io::Result<u64> {
        fs::metadata(&self.path).map(|metadata| metadata.len())
    }
}

impl Drop for OutputGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn terminate_child_group(child: &mut Child) {
    #[cfg(unix)]
    let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}

fn generator_prompt(collector: &StrictCollectorEvidence) -> String {
    format!(
        concat!(
            "Return exactly one JSON object and no markdown. ",
            "Act as the local autonomous Tier 2 generator. ",
            "Propose only concrete missing stability, behavior, or test work supported by cited rows. ",
            "Schema: {{\"proposals\":[{{\"stable_key\":\"kebab-case\",\"title\":\"type: concrete title\",",
            "\"evidence\":[{{\"file\":\"path\",\"line\":1,\"match\":\"exact collector match\"}}],",
            "\"severity\":\"critical|high|medium|low\",\"confidence_millis\":0,",
            "\"complexity\":\"small|medium|large\",\"named_consumer\":\"specific user\"}}]}}. ",
            "Use at most {limit} proposals. Evidence must be copied exactly from this collector: {evidence}"
        ),
        limit = TIER2_RANK_LIMIT,
        evidence = collector_json(collector)
    )
}

fn verifier_prompt(
    collector: &StrictCollectorEvidence,
    generated: &Tier2GeneratedProposals,
    keys: &[String],
) -> String {
    format!(
        concat!(
            "Return exactly one JSON object and no markdown. ",
            "Act as an adversarial Tier 2 verifier. Cover every candidate key exactly once. ",
            "Survive only work that is absent, actionable, and proven by the cited collector row. ",
            "Schema: {{\"verdicts\":[{{\"stable_key\":\"key\",\"verdict\":\"survived|refuted\",",
            "\"reason\":\"bounded evidence-based reason\"}}]}}. ",
            "Required keys: {keys}. Collector: {collector}. Generated: {generated}"
        ),
        keys = json_strings(keys),
        collector = collector_json(collector),
        generated = proposals_json(&generated.proposals)
    )
}

fn collector_json(collector: &StrictCollectorEvidence) -> String {
    let mut remaining = PROMPT_EVIDENCE_LIMIT;
    let mut domains = Vec::new();
    for domain in &collector.domains {
        if remaining == 0 {
            break;
        }
        let rows = domain
            .evidence
            .iter()
            .take(remaining)
            .cloned()
            .collect::<Vec<_>>();
        remaining = remaining.saturating_sub(rows.len());
        if !rows.is_empty() {
            domains.push(format!(
                "{{\"name\":{},\"evidence\":{}}}",
                json_string(&domain.name),
                evidence_json(&rows)
            ));
        }
    }
    let domains = domains.join(",");
    format!("{{\"domains\":[{domains}]}}")
}

fn proposals_json(proposals: &[Tier2Proposal]) -> String {
    let values = proposals
        .iter()
        .map(|proposal| {
            format!(
                "{{\"stable_key\":{},\"title\":{},\"evidence\":{}}}",
                json_string(&proposal.stable_key),
                json_string(&proposal.title),
                evidence_json(&proposal.evidence)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn evidence_json(evidence: &[FileLineEvidence]) -> String {
    let values = evidence
        .iter()
        .map(|row| {
            format!(
                "{{\"file\":{},\"line\":{},\"match\":{}}}",
                json_string(&row.file),
                row.line,
                json_string(&row.r#match)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("[{values}]")
}

fn json_strings(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("strings serialize")
}

fn parse_generated(raw: &str) -> Result<Tier2GeneratedProposals, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw.trim()).map_err(|_| "generator output is not strict JSON")?;
    let proposals = value
        .get("proposals")
        .and_then(serde_json::Value::as_array)
        .ok_or("generator proposals must be an array")?;
    if proposals.len() > usize::try_from(TIER2_RANK_LIMIT).unwrap_or(20) {
        return Err("generator proposal count exceeds the Tier 2 limit".to_string());
    }
    let proposals = proposals
        .iter()
        .map(parse_proposal)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Tier2GeneratedProposals {
        generator_identity: "native-local-generator".to_string(),
        generator_protocol_version: "tier2-v1".to_string(),
        proposals,
    })
}

fn parse_proposal(value: &serde_json::Value) -> Result<Tier2Proposal, String> {
    let object = value.as_object().ok_or("proposal must be an object")?;
    let evidence = object
        .get("evidence")
        .and_then(serde_json::Value::as_array)
        .ok_or("proposal evidence must be an array")?
        .iter()
        .map(parse_evidence)
        .collect::<Result<Vec<_>, _>>()?;
    if evidence.len() > PROPOSAL_EVIDENCE_LIMIT {
        return Err("proposal evidence exceeds the Tier 2 limit".to_string());
    }
    Ok(Tier2Proposal {
        stable_key: required_string(object, "stable_key")?,
        title: required_string(object, "title")?,
        source: Tier2Source::StrictLocalSpecialist,
        evidence,
        severity: match required_string(object, "severity")?.as_str() {
            "critical" => Tier2Severity::Critical,
            "high" => Tier2Severity::High,
            "medium" => Tier2Severity::Medium,
            "low" => Tier2Severity::Low,
            _ => return Err("proposal severity is invalid".to_string()),
        },
        confidence_millis: object
            .get("confidence_millis")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or("proposal confidence_millis is invalid")?,
        complexity: match required_string(object, "complexity")?.as_str() {
            "small" => Tier2Complexity::Small,
            "medium" => Tier2Complexity::Medium,
            "large" => Tier2Complexity::Large,
            _ => return Err("proposal complexity is invalid".to_string()),
        },
        named_consumer: required_string(object, "named_consumer")?,
    })
}

fn parse_evidence(value: &serde_json::Value) -> Result<FileLineEvidence, String> {
    let object = value.as_object().ok_or("evidence must be an object")?;
    Ok(FileLineEvidence {
        file: required_string(object, "file")?,
        line: object
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("evidence line is invalid")?,
        r#match: required_string(object, "match")?,
    })
}

fn parse_verifier(raw: &str) -> Result<Tier2VerifierVerdicts, String> {
    let value: serde_json::Value =
        serde_json::from_str(raw.trim()).map_err(|_| "verifier output is not strict JSON")?;
    let verdicts = value
        .get("verdicts")
        .and_then(serde_json::Value::as_array)
        .ok_or("verifier verdicts must be an array")?
        .iter()
        .map(|value| {
            let object = value.as_object().ok_or("verdict must be an object")?;
            let stable_key = required_string(object, "stable_key")?;
            let reason = required_string(object, "reason")?;
            match required_string(object, "verdict")?.as_str() {
                "survived" => Ok(Tier2Verification::Survived { stable_key, reason }),
                "refuted" => Ok(Tier2Verification::Refuted { stable_key, reason }),
                _ => Err("verdict outcome is invalid".to_string()),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Tier2VerifierVerdicts {
        verifier_identity: "native-local-verifier".to_string(),
        verifier_protocol_version: "tier2-v1".to_string(),
        verdicts,
    })
}

fn required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<String, String> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{field} must be a nonempty string"))
}

fn generator_failure(
    collector: StrictCollectorEvidence,
    code: Tier2FailureCode,
    detail: impl AsRef<str>,
) -> Tier2Scan {
    let failure = bounded_failure(Tier2Stage::Generator, code, detail.as_ref());
    match evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector),
        generator: Tier2StageResult::Failed(failure),
        verifier: Tier2StageResult::Missing,
        roi_policy: Tier2RoiPolicy::v1(),
    }) {
        Err(failure) => Tier2Scan::Failed(failure),
        Ok(_) => unreachable!("failed generator cannot complete"),
    }
}

fn verifier_failure(
    collector: StrictCollectorEvidence,
    generated: Tier2GeneratedProposals,
    detail: impl AsRef<str>,
) -> Tier2Scan {
    let failure = bounded_failure(
        Tier2Stage::Verifier,
        Tier2FailureCode::InvalidVerdictCoverage,
        detail.as_ref(),
    );
    evaluate_scan(collector, generated, Tier2StageResult::Failed(failure))
}

fn evaluate_scan(
    collector: StrictCollectorEvidence,
    generated: Tier2GeneratedProposals,
    verifier: Tier2StageResult<Tier2VerifierVerdicts>,
) -> Tier2Scan {
    match evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Complete(collector),
        generator: Tier2StageResult::Complete(generated),
        verifier,
        roi_policy: Tier2RoiPolicy::v1(),
    }) {
        Ok(evaluation) => Tier2Scan::Complete(
            evaluation
                .observation()
                .cloned()
                .expect("enabled Tier 2 evaluation completes with an observation"),
        ),
        Err(failure) => Tier2Scan::Failed(failure),
    }
}

fn bounded_failure(stage: Tier2Stage, code: Tier2FailureCode, detail: &str) -> Tier2Failure {
    let bounded = detail.chars().take(240).collect::<String>();
    Tier2Failure::new(
        stage,
        code,
        if bounded.trim().is_empty() {
            "native model child returned an empty diagnostic".to_string()
        } else {
            bounded
        },
    )
    .expect("bounded native failure detail is valid")
}

pub(super) fn strict_collector_failure(error: StrictCollectorError) -> Tier2Failure {
    let code = match error.code {
        StrictCollectorErrorCode::InvalidRoot => Tier2FailureCode::InvalidRoot,
        StrictCollectorErrorCode::InvalidCollectorSchema => {
            Tier2FailureCode::InvalidCollectorSchema
        }
        StrictCollectorErrorCode::PathEscapesRoot => Tier2FailureCode::PathEscapesRoot,
        StrictCollectorErrorCode::ReadDirectory => Tier2FailureCode::ReadDirectory,
        StrictCollectorErrorCode::ReadFile => Tier2FailureCode::ReadFile,
        StrictCollectorErrorCode::InvalidUtf8 => Tier2FailureCode::InvalidUtf8,
    };
    let detail = bounded_detail(&error.detail);
    let initial = Tier2Failure::new(Tier2Stage::Collector, code, detail)
        .expect("bounded strict collector detail is valid");
    match evaluate_tier2(Tier2Input::Enabled {
        collector: Tier2StageResult::Failed(initial),
        generator: Tier2StageResult::Missing,
        verifier: Tier2StageResult::Missing,
        roi_policy: Tier2RoiPolicy::v1(),
    }) {
        Err(failure) => failure,
        Ok(_) => unreachable!("a collector failure cannot evaluate successfully"),
    }
}

fn bounded_detail(detail: &str) -> String {
    let bounded = detail.chars().take(240).collect::<String>();
    if bounded.trim().is_empty() {
        "strict collector returned an empty diagnostic".to_string()
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    use autospec_core::autonomous::tier2::Tier2FailureCode;
    use autospec_core::explore::specialists::{StrictCollectorError, StrictCollectorErrorCode};

    use super::strict_collector_failure;

    #[test]
    fn strict_collector_codes_map_exhaustively_to_sealed_tier_two_failures() {
        for (strict, expected) in [
            (
                StrictCollectorErrorCode::InvalidRoot,
                Tier2FailureCode::InvalidRoot,
            ),
            (
                StrictCollectorErrorCode::InvalidCollectorSchema,
                Tier2FailureCode::InvalidCollectorSchema,
            ),
            (
                StrictCollectorErrorCode::PathEscapesRoot,
                Tier2FailureCode::PathEscapesRoot,
            ),
            (
                StrictCollectorErrorCode::ReadDirectory,
                Tier2FailureCode::ReadDirectory,
            ),
            (
                StrictCollectorErrorCode::ReadFile,
                Tier2FailureCode::ReadFile,
            ),
            (
                StrictCollectorErrorCode::InvalidUtf8,
                Tier2FailureCode::InvalidUtf8,
            ),
        ] {
            let failure = strict_collector_failure(StrictCollectorError {
                code: strict,
                detail: "strict collector diagnostic".to_string(),
            });
            assert_eq!(failure.code(), expected);
            assert_eq!(failure.stage().as_str(), "collector");
            assert!(failure.documents().is_some());
        }
    }

    #[test]
    fn strict_collector_oversized_diagnostic_is_bounded_without_becoming_dry() {
        let failure = strict_collector_failure(StrictCollectorError {
            code: StrictCollectorErrorCode::ReadFile,
            detail: "é".repeat(241),
        });

        assert_eq!(failure.detail().chars().count(), 240);
        assert_eq!(failure.status_reason(), "tier2_collector_read_file");
        assert!(failure.documents().is_some());
    }

    #[test]
    fn native_adapter_has_bounded_local_discovery_without_github_mutation_authority() {
        let source = include_str!("tier2_runner.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source before module tests");
        assert!(production.contains("pub(super) fn scan_native"));
        assert!(production.contains("collect_strict_domains"));
        assert!(production.contains("MODEL_TIMEOUT"));
        assert!(production.contains("process_group(0)"));
        let gh_cli = ["\"", "g", "h "].concat();
        for forbidden in [
            "AUTOSPEC_SPECIALIST_LLM_STUB_OUTPUT",
            "scan_specialists",
            "load_or_derive",
            "autospec-explore",
            "sh -c",
            "omx",
            "curl",
            gh_cli.as_str(),
            "github",
            "queue",
            "claim",
            "label",
            "branch",
            "worktree",
            "pull_request",
            "pull request",
            "pr create",
            "issue create",
            "issue edit",
            "issue comment",
            "auto-implement",
            "WaterfallStore",
            "run_foreground",
            "scan_foreground",
            "ExecutorRequest",
            "\"POST\"",
            "\"PATCH\"",
            "\"PUT\"",
            "\"DELETE\"",
            "graphql",
            "pr edit",
            "pr comment",
            "pr merge",
        ] {
            assert!(
                !production.contains(forbidden),
                "Tier 2 disabled-policy adapter retains prohibited authority: {forbidden}"
            );
        }
    }
}
