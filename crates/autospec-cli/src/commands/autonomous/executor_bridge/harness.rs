// executor_bridge/harness.rs — which executor harness runs, and how it is invoked.
//
// Extracted from executor_bridge.rs, which reached 49,482 lines before anything objected and
// is still the largest file in the repository. This cluster leaves cleanly: it answers one
// question — Claude, Codex or OpenCode, configured and aliased how, resolved to which
// invocation — and nothing else in the parent is about that question.
//
// The Codex sandbox policy travels with it because it exists only to decide how a Codex
// invocation is sandboxed, as do the path-safety helpers, which are used nowhere else.

use super::*;

#[cfg(unix)]
mod trusted_executable;
#[cfg(all(unix, test))]
pub(super) use trusted_executable::trusted_executable_owner_allowed;
#[cfg(unix)]
pub(super) use trusted_executable::TrustedExecutable;

#[cfg(test)]
thread_local! {
    static TEST_EXECUTOR_HARNESS_EXACT: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct TestExecutorHarnessGuard(Option<String>);

#[cfg(test)]
impl Drop for TestExecutorHarnessGuard {
    fn drop(&mut self) {
        TEST_EXECUTOR_HARNESS_EXACT.with(|exact| {
            exact.replace(self.0.take());
        });
    }
}

#[cfg(test)]
pub(crate) fn set_test_executor_harness_exact(exact: &str) -> TestExecutorHarnessGuard {
    let previous = TEST_EXECUTOR_HARNESS_EXACT.with(|value| value.replace(Some(exact.into())));
    TestExecutorHarnessGuard(previous)
}

#[cfg(test)]
fn test_executor_harness_exact() -> Option<String> {
    TEST_EXECUTOR_HARNESS_EXACT.with(|exact| exact.borrow().clone())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HarnessKind {
    Claude,
    Codex,
    OpenCode,
    Pi,
}

impl HarnessKind {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::OpenCode),
            "pi" => Ok(Self::Pi),
            other => Err(format!("unsupported executor harness: {other}")),
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HarnessAlias {
    pub(crate) kind: HarnessKind,
    pub(crate) binary: String,
    pub(crate) approval_alias: String,
    pub(crate) display_name: String,
}

#[derive(Clone, Debug)]
pub(crate) struct HarnessConfig {
    pub(super) aliases: Vec<HarnessAlias>,
    pub(super) opencode_adapter: Option<PathBuf>,
}

/// Which model runs a dispatch, independent of which harness runs it.
///
/// This started as two OpenCode-only fields, which meant the two-tier selection
/// in AGENTS.md — and the local-model guardrails in #3344 — applied to one
/// harness out of four. The routing decision is not a property of OpenCode, so
/// it is modelled once here and each harness is asked to express it in its own
/// argv.
///
/// `variant` stays OpenCode-shaped on purpose: it is a reasoning tier, not a
/// model id, and no other harness has an equivalent to generalise over.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ModelRouting {
    pub(super) model: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) context_tokens: Option<u32>,
}

/// Tokens a client carries before any work begins — system prompt, project
/// instructions, memory, skill and MCP schemas — measured from real sessions by
/// `llm/linux-qwen38/scripts/analyze-session-contexts.py` and recorded in
/// `docs/memory/feedback_context_floor_kills_small_tiers.md` (2026-08-18).
///
/// A window below this cannot start a session: it is full before the first
/// question. On a shared KV pool with no admission control that is worse than
/// slow, because the doomed dispatch still reserves its slot and the failure
/// lands on every live session rather than the greedy one. Refusing here is what
/// makes the pool shareable.
///
/// Deliberately absent for Codex and Pi. The source memo is explicit that the
/// floor is client-specific — Claude Code's is nearly 3x OpenCode's because its
/// prompt and skill set are larger — so an unmeasured client gets no floor
/// rather than a borrowed one.
fn measured_context_floor(kind: HarnessKind) -> Option<u32> {
    match kind {
        HarnessKind::Claude => Some(39_655), // floor p50; no p90 measured
        HarnessKind::OpenCode => Some(37_873), // floor p90
        HarnessKind::Codex | HarnessKind::Pi => None,
    }
}

/// Accept `131072` and `128k` alike; an operator writing a budget by hand writes
/// the second. Anything else is refused rather than guessed — a misparsed budget
/// silently becomes a wrong reservation.
fn parse_context_budget(raw: &str) -> Result<u32, String> {
    let text = raw.trim().to_ascii_lowercase();
    let (digits, multiplier) = match text.strip_suffix('k') {
        Some(head) => (head, 1024_u32),
        None => (text.as_str(), 1_u32),
    };
    digits
        .trim()
        .parse::<u32>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            format!(
                "executor_routing_context_unparseable: AUTOSPEC_EXECUTOR_CONTEXT={raw} is not                  a token count; write it as `131072` or `128k`"
            )
        })
}

impl ModelRouting {
    /// `AUTOSPEC_EXECUTOR_MODEL` / `_PROVIDER` are harness-neutral.
    /// `AUTOSPEC_OPENCODE_MODEL` / `_VARIANT` predate them and stay authoritative
    /// for OpenCode: an operator whose configuration names the specific harness
    /// meant that harness, and must not have it silently overridden by a broader
    /// setting. Absent variables leave the harness default untouched.
    fn load(kind: HarnessKind, env: &BTreeMap<String, OsString>) -> Self {
        let read = |key: &str| {
            env.get(key)
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let neutral_model = read("AUTOSPEC_EXECUTOR_MODEL");
        let model = if kind == HarnessKind::OpenCode {
            read("AUTOSPEC_OPENCODE_MODEL").or(neutral_model)
        } else {
            neutral_model
        };
        Self {
            model,
            provider: read("AUTOSPEC_EXECUTOR_PROVIDER"),
            // Named for OpenCode and only ever consumed by it. Reading it for
            // other harnesses would turn an exported variable into a hard
            // failure for a dispatch it was never meant to describe.
            variant: if kind == HarnessKind::OpenCode {
                read("AUTOSPEC_OPENCODE_VARIANT")
            } else {
                None
            },
            context_tokens: None,
        }
    }

    /// Parsed and floor-checked separately from `load` so the failure is a
    /// `Result` the caller surfaces, not a silently dropped field.
    fn with_context(
        mut self,
        kind: HarnessKind,
        env: &BTreeMap<String, OsString>,
    ) -> Result<Self, String> {
        let raw = env
            .get("AUTOSPEC_EXECUTOR_CONTEXT")
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(raw) = raw else {
            return Ok(self);
        };

        let tokens = parse_context_budget(raw)?;
        if let Some(floor) = measured_context_floor(kind) {
            if tokens < floor {
                return Err(format!(
                    "executor_routing_context_below_floor: {tokens} tokens is under the                      {floor}-token session floor measured for {}; a window this small is                      full before the first question and would hold a slot in the shared KV                      pool without ever starting (docs/memory/\
                     feedback_context_floor_kills_small_tiers.md)",
                    kind.as_str()
                ));
            }
        }
        self.context_tokens = Some(tokens);
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedHarness {
    pub(crate) kind: HarnessKind,
    pub(crate) executable: PathBuf,
    pub(super) opencode_adapter: Option<PathBuf>,
    pub(super) codex_sandbox: CodexSandboxPolicy,
    pub(super) routing: ModelRouting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HarnessInvocation {
    pub(crate) program: PathBuf,
    pub(crate) supervised_executable: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) current_dir: PathBuf,
    pub(crate) requires_mutation_snapshots: bool,
}

impl HarnessConfig {
    pub(crate) fn load(repo: &Path, env: &BTreeMap<String, OsString>) -> Result<Self, String> {
        let table = alias_table_path(repo, env)?;
        let body = fs::read_to_string(&table)
            .map_err(|error| format!("read harness alias table: {error}"))?;
        let aliases = Self::parse_alias_table(&body)?;
        let opencode_adapter = match env.get("AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER") {
            Some(path) => Some(safe_executable(Path::new(path), env)?),
            None => default_opencode_adapter(env).transpose()?,
        };
        Ok(Self {
            aliases,
            opencode_adapter,
        })
    }

    pub(crate) fn parse_alias_table(body: &str) -> Result<Vec<HarnessAlias>, String> {
        body.lines()
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .map(|line| {
                let columns: Vec<_> = line.split('\t').collect();
                if columns.len() != 4 {
                    return Err(format!(
                        "harness alias row must contain exactly four TSV columns: {line}"
                    ));
                }
                Ok(HarnessAlias {
                    kind: HarnessKind::parse(columns[0])?,
                    binary: columns[1].trim().to_string(),
                    approval_alias: columns[2].trim().to_string(),
                    display_name: columns[3].trim().to_string(),
                })
            })
            .collect()
    }

    pub(crate) fn resolve(
        &self,
        env: &BTreeMap<String, OsString>,
    ) -> Result<ResolvedHarness, String> {
        let requested = env
            .get("AUTOSPEC_HANDOFF_DISPATCHER_KIND")
            .and_then(|value| value.to_str())
            .map(HarnessKind::parse)
            .transpose()?
            .or_else(|| {
                if env.contains_key("CODEX_THREAD_ID") || env.contains_key("CODEX_CI") {
                    Some(HarnessKind::Codex)
                } else if env.contains_key("CLAUDECODE")
                    || env.contains_key("CLAUDE_CODE_ENTRYPOINT")
                {
                    Some(HarnessKind::Claude)
                } else if env.contains_key("OPENCODE") || env.contains_key("OPENCODE_SESSION_ID") {
                    Some(HarnessKind::OpenCode)
                } else {
                    None
                }
            });
        if let Some(requested) = requested {
            return self.resolve_kind(requested, env);
        }

        for alias in &self.aliases {
            let binary = Path::new(&alias.binary);
            if is_bare_path(binary) && !bare_path_exists(binary, env) {
                continue;
            }
            return self.resolve_kind(alias.kind, env);
        }
        Err("executor_harness_unknown: no configured harness is available on PATH".to_string())
    }

    fn resolve_kind(
        &self,
        requested: HarnessKind,
        env: &BTreeMap<String, OsString>,
    ) -> Result<ResolvedHarness, String> {
        let alias = self
            .aliases
            .iter()
            .find(|alias| alias.kind == requested)
            .ok_or_else(|| format!("executor harness alias missing: {}", requested.as_str()))?;
        Ok(ResolvedHarness {
            kind: requested,
            executable: safe_executable(Path::new(&alias.binary), env)?,
            opencode_adapter: self.opencode_adapter.clone(),
            codex_sandbox: CodexSandboxPolicy::Default,
            routing: ModelRouting::load(requested, env).with_context(requested, env)?,
        })
    }
}

impl ResolvedHarness {
    /// The routing flags this harness's CLI accepts, to be spliced in
    /// immediately before the positional prompt. Every one of these CLIs treats
    /// a flag after the prompt as part of the prompt, so position is part of the
    /// contract, not a formatting preference.
    ///
    /// A directive the harness cannot express is an error rather than a silent
    /// drop. Emitting the model while discarding the provider would let the
    /// routing ledger record a dispatch as routed when the argv never carried
    /// the routing — the precise failure the #3344 guardrails exist to catch.
    fn routing_args(&self) -> Result<Vec<String>, String> {
        let mut args = Vec::new();

        if let Some(provider) = &self.routing.provider {
            match self.kind {
                // Pi selects the endpoint by provider name; see its --provider flag.
                HarnessKind::Pi => {
                    args.push("--provider".into());
                    args.push(provider.clone());
                }
                // Claude, Codex and OpenCode bind a provider through their own
                // configuration, not an invocation flag.
                other => {
                    return Err(format!(
                        "executor_routing_provider_unsupported: {} cannot express \
                         AUTOSPEC_EXECUTOR_PROVIDER={provider} as an invocation flag; \
                         configure the provider in the harness itself or dispatch \
                         through a harness that accepts one",
                        other.as_str()
                    ));
                }
            }
        }

        if let Some(model) = &self.routing.model {
            args.push("--model".into());
            args.push(model.clone());
        }

        // Only Codex exposes the window as a per-invocation setting. The others
        // take it from the model or the server, so the budget cannot reach their
        // argv — and unlike a provider directive, that is not a
        // misrepresentation: a context budget asserts capacity, it does not
        // redirect the work somewhere the caller did not ask for. Its real
        // enforcement is the floor check in `ModelRouting::with_context`, which
        // has already run by the time we get here.
        if let Some(tokens) = self.routing.context_tokens {
            if self.kind == HarnessKind::Codex {
                args.push("-c".into());
                args.push(format!("model_context_window={tokens}"));
            }
        }

        if let Some(variant) = &self.routing.variant {
            match self.kind {
                HarnessKind::OpenCode => {
                    args.push("--variant".into());
                    args.push(variant.clone());
                }
                other => {
                    return Err(format!(
                        "executor_routing_variant_unsupported: {} has no reasoning-variant \
                         flag; AUTOSPEC_OPENCODE_VARIANT applies to OpenCode only",
                        other.as_str()
                    ));
                }
            }
        }

        Ok(args)
    }
    pub(super) fn review_invocation(
        &self,
        worktree: &Path,
        artifact: &Path,
        prompt: &str,
    ) -> Result<HarnessInvocation, String> {
        let routing = self.routing_args()?;
        let (program, args) = match self.kind {
            HarnessKind::Codex => (
                self.executable.clone(),
                vec![
                    "exec".into(),
                    "--ignore-user-config".into(),
                    "--ignore-rules".into(),
                    "-C".into(),
                    worktree.display().to_string(),
                    "--sandbox".into(),
                    "read-only".into(),
                    "--ephemeral".into(),
                    "--output-last-message".into(),
                    artifact.display().to_string(),
                ]
                .into_iter()
                .chain(routing.clone())
                .chain(std::iter::once(prompt.to_string()))
                .collect(),
            ),
            HarnessKind::Claude => (
                self.executable.clone(),
                vec![
                    "-p".into(),
                    "--tools".into(),
                    CLAUDE_REVIEW_TOOLS.into(),
                    "--safe-mode".into(),
                    "--disable-slash-commands".into(),
                    "--setting-sources".into(),
                    String::new(),
                    "--strict-mcp-config".into(),
                    "--mcp-config".into(),
                    r#"{"mcpServers":{}}"#.into(),
                    "--permission-mode".into(),
                    "plan".into(),
                    "--allowedTools".into(),
                    CLAUDE_REVIEW_TOOLS.into(),
                    "--disallowedTools".into(),
                    "Edit,Write,Bash".into(),
                    "--no-session-persistence".into(),
                    "--output-format".into(),
                    "text".into(),
                ]
                .into_iter()
                .chain(routing.clone())
                .chain(std::iter::once(prompt.to_string()))
                .collect(),
            ),
            HarnessKind::OpenCode => {
                let mut args = vec![
                    "--pure".into(),
                    "run".into(),
                    "--agent".into(),
                    OPENCODE_REVIEWER_AGENT.into(),
                ];
                args.extend(routing.clone());
                args.push(prompt.into());
                (self.executable.clone(), args)
            }
            HarnessKind::Pi => {
                let mut args = routing.clone();
                args.push(prompt.into());
                (self.executable.clone(), args)
            }
        };
        Ok(HarnessInvocation {
            program,
            supervised_executable: self.executable.clone(),
            args,
            current_dir: worktree.to_path_buf(),
            requires_mutation_snapshots: false,
        })
    }

    pub(crate) fn invocation(
        &self,
        worktree: &Path,
        artifact: &Path,
        prompt: &str,
    ) -> Result<HarnessInvocation, String> {
        #[cfg(test)]
        if let Some(exact_test) = test_executor_harness_exact() {
            return Ok(HarnessInvocation {
                program: self.executable.clone(),
                supervised_executable: self.executable.clone(),
                args: vec![
                    "--exact".into(),
                    exact_test,
                    "--ignored".into(),
                    "--nocapture".into(),
                ],
                current_dir: worktree.to_path_buf(),
                requires_mutation_snapshots: false,
            });
        }
        let routing = self.routing_args()?;
        let (program, args, requires_mutation_snapshots) = match self.kind {
            HarnessKind::Codex => {
                let mut args = vec!["exec".into(), "-C".into(), worktree.display().to_string()];
                if self.codex_sandbox == CodexSandboxPolicy::Default {
                    args.extend(["--sandbox".into(), "workspace-write".into()]);
                } else {
                    args.extend(self.codex_sandbox.invocation_args(&self.executable)?);
                }
                args.extend([
                    "--ephemeral".into(),
                    "--output-last-message".into(),
                    artifact.display().to_string(),
                ]);
                args.extend(routing.clone());
                args.push(prompt.into());
                (self.executable.clone(), args, false)
            }
            HarnessKind::Claude => (
                self.executable.clone(),
                vec![
                    "-p".into(),
                    "--tools".into(),
                    CLAUDE_BUILTIN_TOOLS.into(),
                    "--safe-mode".into(),
                    "--disable-slash-commands".into(),
                    "--setting-sources".into(),
                    String::new(),
                    "--strict-mcp-config".into(),
                    "--mcp-config".into(),
                    r#"{"mcpServers":{}}"#.into(),
                    "--permission-mode".into(),
                    "acceptEdits".into(),
                    "--allowedTools".into(),
                    CLAUDE_LOCAL_TOOLS.into(),
                    "--disallowedTools".into(),
                    CLAUDE_FORBIDDEN_TOOLS.into(),
                    "--no-session-persistence".into(),
                    // Text mode buffers until exit, so long tool runs appear idle to supervision.
                    "--verbose".into(),
                    "--include-partial-messages".into(),
                    "--output-format".into(),
                    "stream-json".into(),
                ]
                .into_iter()
                .chain(routing.clone())
                .chain(std::iter::once(prompt.to_string()))
                .collect(),
                true,
            ),
            HarnessKind::OpenCode => {
                let adapter = self
                    .opencode_adapter
                    .clone()
                    .ok_or_else(|| "executor_harness_uncontained: OpenCode requires AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string())?;
                let mut args = vec![
                    self.executable.display().to_string(),
                    "--pure".into(),
                    "run".into(),
                ];
                args.extend(routing.clone());
                args.push(prompt.into());
                (adapter, args, false)
            }
            HarnessKind::Pi => {
                let mut args = routing.clone();
                args.push(prompt.into());
                (self.executable.clone(), args, false)
            }
        };
        Ok(HarnessInvocation {
            program,
            supervised_executable: self.executable.clone(),
            args,
            current_dir: worktree.to_path_buf(),
            requires_mutation_snapshots,
        })
    }
}

pub(super) fn safe_executable(
    path: &Path,
    env: &BTreeMap<String, OsString>,
) -> Result<PathBuf, String> {
    if !is_bare_path(path) && !path.is_absolute() {
        return Err(format!(
            "executor harness path must be absolute or a bare PATH alias: {}",
            path.display()
        ));
    }
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env.get("PATH")
            .and_then(|value| value.to_str())
            .into_iter()
            .flat_map(std::env::split_paths)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(path))
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| format!("executor harness not found on PATH: {}", path.display()))?
    };
    #[cfg(test)]
    let allow_current_test_executable = test_executor_harness_exact().is_some()
        && fs::canonicalize(&candidate).ok()
            == std::env::current_exe()
                .ok()
                .and_then(|path| fs::canonicalize(path).ok());
    #[cfg(not(test))]
    let allow_current_test_executable = false;
    if temporary_path(&candidate, env) && !allow_current_test_executable {
        return Err(format!(
            "executor harness is configured through temporary storage: {}",
            candidate.display()
        ));
    }
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "canonicalize executor harness {}: {error}",
            candidate.display()
        )
    })?;
    if temporary_path(&canonical, env) && !allow_current_test_executable {
        return Err(format!(
            "executor harness resolves through temporary storage: {}",
            canonical.display()
        ));
    }
    if !canonical.is_absolute() || !canonical.is_file() {
        return Err(format!(
            "executor harness is not an absolute regular file: {}",
            canonical.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(&canonical)
            .map_err(|error| {
                format!(
                    "read executor harness metadata {}: {error}",
                    canonical.display()
                )
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(format!(
                "executor harness is not executable: {}",
                canonical.display()
            ));
        }
    }
    Ok(canonical)
}

pub(super) fn temporary_path(path: &Path, env: &BTreeMap<String, OsString>) -> bool {
    const TEMPORARY_ROOTS: [&str; 5] = [
        "/tmp",
        "/private/tmp",
        "/var/tmp",
        "/private/var/tmp",
        "/var/folders",
    ];

    TEMPORARY_ROOTS.iter().any(|root| path.starts_with(root))
        || env
            .get("TMPDIR")
            .filter(|root| !root.is_empty())
            .is_some_and(|root| path.starts_with(Path::new(root)))
}

/// Resolve the OpenCode containment adapter when the operator has not set
/// `AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER` explicitly. Falls back to the shipped
/// `scripts/lib/opencode-containment-adapter.sh` under `AUTOSPEC_SCRIPTS_DIR`
/// or `~/.autospec/scripts`. Returns `None` when neither exists — the mutating
/// OpenCode implementer then fails closed with `executor_harness_uncontained`.
fn default_opencode_adapter(env: &BTreeMap<String, OsString>) -> Option<Result<PathBuf, String>> {
    let mut candidates = Vec::new();
    if let Some(scripts_dir) = env.get("AUTOSPEC_SCRIPTS_DIR") {
        candidates.push(PathBuf::from(scripts_dir).join("lib/opencode-containment-adapter.sh"));
    }
    if let Some(home) = env.get("HOME") {
        candidates.push(
            PathBuf::from(home).join(".autospec/scripts/lib/opencode-containment-adapter.sh"),
        );
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Some(safe_executable(&candidate, env));
        }
    }
    None
}

fn alias_table_path(repo: &Path, env: &BTreeMap<String, OsString>) -> Result<PathBuf, String> {
    if let Some(path) = env.get("AUTOSPEC_HARNESS_RUNTIME_ALIASES") {
        return Ok(PathBuf::from(path));
    }
    if let Some(config_dir) = env.get("AUTOSPEC_CONFIG_DIR") {
        let path = PathBuf::from(config_dir).join("harness-runtime-aliases.tsv");
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "installed harness alias table not found: {}",
            path.display()
        ));
    }

    let mut candidates = Vec::new();
    if let Some(home) = env.get("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join(".autospec/config")
                .join("harness-runtime-aliases.tsv"),
        );
    }
    candidates.push(repo.join("config/harness-runtime-aliases.tsv"));
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "installed harness alias table not found; set AUTOSPEC_HARNESS_RUNTIME_ALIASES or AUTOSPEC_CONFIG_DIR".to_string()
        })
}

fn is_bare_path(path: &Path) -> bool {
    path.components().count() == 1
}

fn bare_path_exists(path: &Path, env: &BTreeMap<String, OsString>) -> bool {
    env.get("PATH")
        .and_then(|value| value.to_str())
        .into_iter()
        .flat_map(std::env::split_paths)
        .filter(|directory| directory.is_absolute())
        .any(|directory| directory.join(path).is_file())
}
