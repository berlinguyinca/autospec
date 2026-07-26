use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const CLAUDE_BUILTIN_TOOLS: &str = "Read,Edit,Write,Glob,Grep,Bash";
pub(crate) const CLAUDE_LOCAL_TOOLS: &str = concat!(
    "Read,Edit,Write,Glob,Grep,",
    "Bash(git status),Bash(git status *),Bash(git diff),Bash(git diff *),",
    "Bash(git add *),Bash(git commit *),",
    "Bash(cargo test),Bash(cargo test *),Bash(cargo check),Bash(cargo check *),",
    "Bash(cargo clippy),Bash(cargo clippy *),",
    "Bash(npm test),Bash(npm test *),Bash(npm run test),Bash(npm run test *),",
    "Bash(pnpm test),Bash(pnpm test *),Bash(pnpm run test),Bash(pnpm run test *),",
    "Bash(yarn test),Bash(yarn test *),Bash(yarn run test),Bash(yarn run test *),",
    "Bash(go test),Bash(go test *),Bash(pytest),Bash(pytest *),",
    "Bash(python -m pytest),Bash(python -m pytest *),",
    "Bash(mvn test),Bash(mvn test *),Bash(./mvnw test),Bash(./mvnw test *),",
    "Bash(gradle test),Bash(gradle test *),Bash(./gradlew test),",
    "Bash(./gradlew test *),Bash(sbt test),Bash(sbt test *),",
    "Bash(make test),Bash(make test *)"
);
pub(crate) const CLAUDE_FORBIDDEN_TOOLS: &str = concat!(
    "Bash(git push),Bash(git push *),Bash(git fetch),Bash(git fetch *),",
    "Bash(git pull),Bash(git pull *),Bash(git remote),Bash(git remote *),",
    "Bash(git worktree),Bash(git worktree *),Bash(git branch),Bash(git branch *),",
    "Bash(git checkout),Bash(git checkout *),Bash(git switch),Bash(git switch *),",
    "Bash(git merge),Bash(git merge *),Bash(git rebase),Bash(git rebase *),",
    "Bash(git reset),Bash(git reset *),Bash(git clean),Bash(git clean *),",
    "Bash(git clone),Bash(git clone *),Bash(git init),Bash(git init *),",
    "Bash(git commit *--amend*)"
);
const INVOCATION_SCHEMA: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HarnessKind {
    Claude,
    Codex,
    OpenCode,
}

impl HarnessKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::OpenCode),
            other => Err(format!("unsupported executor harness: {other}")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
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
    aliases: Vec<HarnessAlias>,
    opencode_adapter: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedHarness {
    pub(crate) kind: HarnessKind,
    pub(crate) executable: PathBuf,
    opencode_adapter: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HarnessInvocation {
    pub(crate) program: PathBuf,
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
        let opencode_adapter = env
            .get("AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER")
            .map(PathBuf::from)
            .map(|path| safe_executable(&path, env))
            .transpose()?;
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
        })
    }
}

impl ResolvedHarness {
    pub(crate) fn invocation(
        &self,
        worktree: &Path,
        artifact: &Path,
        prompt: &str,
    ) -> Result<HarnessInvocation, String> {
        let (program, args, requires_mutation_snapshots) = match self.kind {
            HarnessKind::Codex => (
                self.executable.clone(),
                vec![
                    "exec".into(),
                    "-C".into(),
                    worktree.display().to_string(),
                    "--sandbox".into(),
                    "workspace-write".into(),
                    "--ephemeral".into(),
                    "--output-last-message".into(),
                    artifact.display().to_string(),
                    prompt.into(),
                ],
                false,
            ),
            HarnessKind::Claude => (
                self.executable.clone(),
                vec![
                    "-p".into(),
                    "--tools".into(),
                    CLAUDE_BUILTIN_TOOLS.into(),
                    "--safe-mode".into(),
                    "--bare".into(),
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
                    "--output-format".into(),
                    "text".into(),
                    prompt.into(),
                ],
                true,
            ),
            HarnessKind::OpenCode => {
                let adapter = self
                    .opencode_adapter
                    .clone()
                    .ok_or_else(|| "executor_harness_uncontained: OpenCode requires AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string())?;
                (
                    adapter,
                    vec![
                        self.executable.display().to_string(),
                        "--pure".into(),
                        "run".into(),
                        prompt.into(),
                    ],
                    false,
                )
            }
        };
        Ok(HarnessInvocation {
            program,
            args,
            current_dir: worktree.to_path_buf(),
            requires_mutation_snapshots,
        })
    }
}

fn safe_executable(path: &Path, env: &BTreeMap<String, OsString>) -> Result<PathBuf, String> {
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
    if temporary_path(&candidate, env) {
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
    if temporary_path(&canonical, env) {
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

fn temporary_path(path: &Path, env: &BTreeMap<String, OsString>) -> bool {
    const TEMPORARY_ROOTS: [&str; 4] = ["/tmp", "/private/tmp", "/var/tmp", "/var/folders"];

    TEMPORARY_ROOTS.iter().any(|root| path.starts_with(root))
        || env
            .get("TMPDIR")
            .filter(|root| !root.is_empty())
            .is_some_and(|root| path.starts_with(Path::new(root)))
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BridgeIdentity {
    pub(crate) repository: String,
    pub(crate) issue: u64,
    pub(crate) worker_id: String,
    pub(crate) branch: String,
    pub(crate) claim_id: String,
    pub(crate) invocation_id: String,
    pub(crate) base_ref: String,
    pub(crate) base_oid: String,
    pub(crate) worktree: PathBuf,
    pub(crate) runtime_session_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgePhase {
    Implementing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) process_group: u32,
    pub(crate) executable: PathBuf,
    pub(crate) argv_digest: String,
    pub(crate) boot_id: String,
    pub(crate) start_identity: String,
}

impl ProcessIdentity {
    pub(crate) fn matches(&self, observed: &Self) -> bool {
        self == observed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PersistedInvocation {
    pub(crate) schema: u32,
    pub(crate) identity: BridgeIdentity,
    pub(crate) harness: HarnessKind,
    pub(crate) phase: BridgePhase,
    pub(crate) process: Option<ProcessIdentity>,
    pub(crate) progress_at: u64,
    pub(crate) pr: Option<u64>,
    pub(crate) head_oid: Option<String>,
    pub(crate) terminal_result: Option<String>,
}

impl PersistedInvocation {
    pub(crate) fn to_json(&self) -> Result<String, String> {
        Ok(invocation_to_value(self).to_string())
    }

    pub(crate) fn from_json(body: &str) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| format!("invalid invocation JSON: {error}"))?;
        invocation_from_value(value)
    }
}

fn invocation_to_value(invocation: &PersistedInvocation) -> serde_json::Value {
    let identity = &invocation.identity;
    let process = invocation.process.as_ref().map(|process| {
        serde_json::json!({
            "pid": process.pid,
            "process_group": process.process_group,
            "executable": process.executable,
            "argv_digest": process.argv_digest,
            "boot_id": process.boot_id,
            "start_identity": process.start_identity,
        })
    });
    serde_json::json!({
        "schema": invocation.schema,
        "identity": {
            "repository": identity.repository,
            "issue": identity.issue,
            "worker_id": identity.worker_id,
            "branch": identity.branch,
            "claim_id": identity.claim_id,
            "invocation_id": identity.invocation_id,
            "base_ref": identity.base_ref,
            "base_oid": identity.base_oid,
            "worktree": identity.worktree,
            "runtime_session_id": identity.runtime_session_id,
        },
        "harness": invocation.harness.as_str(),
        "phase": "implementing",
        "process": process,
        "progress_at": invocation.progress_at,
        "pr": invocation.pr,
        "head_oid": invocation.head_oid,
        "terminal_result": invocation.terminal_result,
    })
}

fn invocation_from_value(value: serde_json::Value) -> Result<PersistedInvocation, String> {
    let object = strict_object(
        value,
        &[
            "schema",
            "identity",
            "harness",
            "phase",
            "process",
            "progress_at",
            "pr",
            "head_oid",
            "terminal_result",
        ],
        "invocation",
    )?;
    let identity = strict_object(
        required(&object, "identity")?.clone(),
        &[
            "repository",
            "issue",
            "worker_id",
            "branch",
            "claim_id",
            "invocation_id",
            "base_ref",
            "base_oid",
            "worktree",
            "runtime_session_id",
        ],
        "identity",
    )?;
    let process = match required(&object, "process")? {
        serde_json::Value::Null => None,
        value => {
            let process = strict_object(
                value.clone(),
                &[
                    "pid",
                    "process_group",
                    "executable",
                    "argv_digest",
                    "boot_id",
                    "start_identity",
                ],
                "process",
            )?;
            Some(ProcessIdentity {
                pid: checked_u32(&process, "pid")?,
                process_group: checked_u32(&process, "process_group")?,
                executable: PathBuf::from(text(&process, "executable")?),
                argv_digest: text(&process, "argv_digest")?,
                boot_id: text(&process, "boot_id")?,
                start_identity: text(&process, "start_identity")?,
            })
        }
    };
    let phase = text(&object, "phase")?;
    if phase != "implementing" {
        return Err(format!("unsupported bridge phase: {phase}"));
    }
    let schema = checked_u32(&object, "schema")?;
    if schema != INVOCATION_SCHEMA {
        return Err(format!("unsupported invocation schema: {schema}"));
    }
    Ok(PersistedInvocation {
        schema,
        identity: BridgeIdentity {
            repository: text(&identity, "repository")?,
            issue: number(&identity, "issue")?,
            worker_id: text(&identity, "worker_id")?,
            branch: text(&identity, "branch")?,
            claim_id: text(&identity, "claim_id")?,
            invocation_id: text(&identity, "invocation_id")?,
            base_ref: text(&identity, "base_ref")?,
            base_oid: text(&identity, "base_oid")?,
            worktree: PathBuf::from(text(&identity, "worktree")?),
            runtime_session_id: optional_text(&identity, "runtime_session_id")?,
        },
        harness: HarnessKind::parse(&text(&object, "harness")?)?,
        phase: BridgePhase::Implementing,
        process,
        progress_at: number(&object, "progress_at")?,
        pr: optional_number(&object, "pr")?,
        head_oid: optional_text(&object, "head_oid")?,
        terminal_result: optional_text(&object, "terminal_result")?,
    })
}

type JsonObject = serde_json::Map<String, serde_json::Value>;

fn strict_object(
    value: serde_json::Value,
    allowed: &[&str],
    label: &str,
) -> Result<JsonObject, String> {
    let object = value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} must be an object"))?;
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("unexpected field in {label}: {field}"));
    }
    for field in allowed {
        if !object.contains_key(*field) {
            return Err(format!("missing field in {label}: {field}"));
        }
    }
    Ok(object)
}

fn required<'a>(object: &'a JsonObject, field: &str) -> Result<&'a serde_json::Value, String> {
    object
        .get(field)
        .ok_or_else(|| format!("missing field: {field}"))
}

fn text(object: &JsonObject, field: &str) -> Result<String, String> {
    required(object, field)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{field} must be a string"))
}

fn number(object: &JsonObject, field: &str) -> Result<u64, String> {
    required(object, field)?
        .as_u64()
        .ok_or_else(|| format!("{field} must be an unsigned integer"))
}

fn checked_u32(object: &JsonObject, field: &str) -> Result<u32, String> {
    u32::try_from(number(object, field)?).map_err(|_| format!("{field} is out of range for u32"))
}

fn optional_text(object: &JsonObject, field: &str) -> Result<Option<String>, String> {
    match required(object, field)? {
        serde_json::Value::Null => Ok(None),
        value => value
            .as_str()
            .map(|value| Some(value.to_string()))
            .ok_or_else(|| format!("{field} must be a string or null")),
    }
}

fn optional_number(object: &JsonObject, field: &str) -> Result<Option<u64>, String> {
    match required(object, field)? {
        serde_json::Value::Null => Ok(None),
        value => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{field} must be an unsigned integer or null")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        BridgeIdentity, BridgePhase, HarnessConfig, HarnessKind, PersistedInvocation,
        ProcessIdentity, CLAUDE_BUILTIN_TOOLS, CLAUDE_FORBIDDEN_TOOLS, CLAUDE_LOCAL_TOOLS,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn test_root(label: &str) -> PathBuf {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-autonomous-executor-bridge-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create executor bridge test root");
        root
    }

    fn write_alias_table(root: &Path, body: &str) -> PathBuf {
        let path = root.join("harness-runtime-aliases.tsv");
        fs::write(&path, body).expect("write harness alias table");
        path
    }

    fn environment(table: &Path) -> BTreeMap<String, OsString> {
        BTreeMap::from([
            (
                "AUTOSPEC_HARNESS_RUNTIME_ALIASES".to_string(),
                table.as_os_str().to_os_string(),
            ),
            ("PATH".to_string(), OsString::from("/bin:/usr/bin")),
        ])
    }

    fn installed_aliases() -> &'static str {
        "claude\ttrue\t--dangerously-skip-permissions\tClaude Code\n\
         codex\tsh\t--yolo\tCodex CLI\n\
         opencode\tfalse\t\tOpenCode\n"
    }

    #[test]
    fn autonomous_executor_bridge_runtime_marker_precedes_alias_path_order() {
        // Break caught: probing the first installed binary before the active runtime marker.
        let root = test_root("marker");
        let table = write_alias_table(&root, installed_aliases());
        let mut env = environment(&table);
        env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));

        let config = HarnessConfig::load(&root, &env).expect("load harness config");
        let resolved = config.resolve(&env).expect("resolve active harness");

        assert_eq!(resolved.kind, HarnessKind::Codex);
        assert_eq!(
            resolved.executable,
            fs::canonicalize("/bin/sh").expect("canonical /bin/sh")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn autonomous_executor_bridge_explicit_override_precedes_runtime_marker() {
        // Break caught: a Codex marker overriding an operator's explicit Claude selection.
        let root = test_root("override");
        let table = write_alias_table(&root, installed_aliases());
        let mut env = environment(&table);
        env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));
        env.insert(
            "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
            OsString::from("claude"),
        );

        let resolved = HarnessConfig::load(&root, &env)
            .expect("load harness config")
            .resolve(&env)
            .expect("resolve explicit harness");

        assert_eq!(resolved.kind, HarnessKind::Claude);
        assert_eq!(
            resolved.executable,
            fs::canonicalize("/bin/true").expect("canonical /bin/true")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn autonomous_executor_bridge_alias_table_parses_all_four_columns() {
        // Break caught: hard-coded harness binaries or discarded approval/display metadata.
        let aliases = HarnessConfig::parse_alias_table(installed_aliases())
            .expect("parse installed alias table");

        assert_eq!(aliases.len(), 3);
        assert_eq!(aliases[0].kind, HarnessKind::Claude);
        assert_eq!(aliases[0].binary, "true");
        assert_eq!(aliases[0].approval_alias, "--dangerously-skip-permissions");
        assert_eq!(aliases[0].display_name, "Claude Code");
        assert_eq!(aliases[2].kind, HarnessKind::OpenCode);
        assert_eq!(aliases[2].approval_alias, "");
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_executor_bridge_rejects_temporary_dispatcher_paths() {
        // Break caught: launching a PATH-shadowed dispatcher from an attacker-writable temp root.
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("unsafe-dispatcher");
        let dispatcher = root.join("codex");
        fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write dispatcher");
        fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755))
            .expect("make dispatcher executable");
        let table = write_alias_table(
            &root,
            "codex\tcodex\t--yolo\tCodex CLI\n\
             claude\ttrue\t--dangerously-skip-permissions\tClaude Code\n\
             opencode\tfalse\t\tOpenCode\n",
        );
        let mut env = environment(&table);
        env.insert("PATH".to_string(), root.as_os_str().to_os_string());
        env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));

        let error = HarnessConfig::load(&root, &env)
            .expect("load harness config")
            .resolve(&env)
            .expect_err("temporary dispatcher must fail closed");

        assert!(error.contains("temporary"), "unexpected error: {error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn autonomous_executor_bridge_builds_exact_codex_arguments() {
        // Break caught: Codex launching without workspace-write containment or a result artifact.
        let root = test_root("codex-argv");
        let table = write_alias_table(&root, installed_aliases());
        let mut env = environment(&table);
        env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));
        let resolved = HarnessConfig::load(&root, &env)
            .expect("load harness config")
            .resolve(&env)
            .expect("resolve Codex");
        let worktree = Path::new("/safe/worktree");
        let artifact = Path::new("/safe/state/last-message.txt");

        let invocation = resolved
            .invocation(worktree, artifact, "implement issue 42")
            .expect("build Codex invocation");

        assert_eq!(invocation.program, resolved.executable);
        assert_eq!(invocation.current_dir, worktree);
        assert_eq!(
            invocation.args,
            vec![
                "exec",
                "-C",
                "/safe/worktree",
                "--sandbox",
                "workspace-write",
                "--ephemeral",
                "--output-last-message",
                "/safe/state/last-message.txt",
                "implement issue 42",
            ]
        );
        assert!(!invocation.requires_mutation_snapshots);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn autonomous_executor_bridge_builds_exact_claude_arguments() {
        // Break caught: Claude being unable to test/commit locally, or inheriting remote Git
        // permissions from user/project settings.
        let root = test_root("claude-argv");
        let table = write_alias_table(&root, installed_aliases());
        let mut env = environment(&table);
        env.insert(
            "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
            OsString::from("claude"),
        );
        let resolved = HarnessConfig::load(&root, &env)
            .expect("load harness config")
            .resolve(&env)
            .expect("resolve Claude");

        let invocation = resolved
            .invocation(
                Path::new("/safe/worktree"),
                Path::new("/safe/state/unused.txt"),
                "implement issue 42",
            )
            .expect("build Claude invocation");

        assert_eq!(
            invocation.args,
            vec![
                "-p",
                "--tools",
                CLAUDE_BUILTIN_TOOLS,
                "--safe-mode",
                "--bare",
                "--disable-slash-commands",
                "--setting-sources",
                "",
                "--strict-mcp-config",
                "--mcp-config",
                r#"{"mcpServers":{}}"#,
                "--permission-mode",
                "acceptEdits",
                "--allowedTools",
                CLAUDE_LOCAL_TOOLS,
                "--disallowedTools",
                CLAUDE_FORBIDDEN_TOOLS,
                "--no-session-persistence",
                "--output-format",
                "text",
                "implement issue 42",
            ]
        );
        assert!(invocation.requires_mutation_snapshots);
        assert!(!invocation
            .args
            .iter()
            .any(|arg| arg == "--dangerously-skip-permissions"));
        assert_eq!(
            CLAUDE_BUILTIN_TOOLS, "Read,Edit,Write,Glob,Grep,Bash",
            "Claude must expose only the six local built-ins needed for implementation"
        );
        for isolation_flag in [
            "--safe-mode",
            "--bare",
            "--disable-slash-commands",
            "--strict-mcp-config",
        ] {
            assert!(
                invocation.args.iter().any(|arg| arg == isolation_flag),
                "Claude invocation must include {isolation_flag}"
            );
        }
        let strict_mcp = invocation
            .args
            .windows(2)
            .find(|pair| pair[0] == "--mcp-config")
            .expect("explicit MCP configuration");
        assert_eq!(strict_mcp[1], r#"{"mcpServers":{}}"#);
        let setting_sources = invocation
            .args
            .windows(2)
            .find(|pair| pair[0] == "--setting-sources")
            .expect("explicit setting source isolation");
        assert_eq!(setting_sources[1], "");
        for required in [
            "Read",
            "Edit",
            "Write",
            "Glob",
            "Grep",
            "Bash(git status)",
            "Bash(git status *)",
            "Bash(git diff)",
            "Bash(git diff *)",
            "Bash(git add *)",
            "Bash(git commit *)",
            "Bash(cargo test)",
            "Bash(cargo test *)",
            "Bash(npm test)",
            "Bash(npm test *)",
            "Bash(pnpm test *)",
            "Bash(yarn test *)",
            "Bash(go test *)",
            "Bash(pytest *)",
            "Bash(python -m pytest *)",
        ] {
            assert!(
                CLAUDE_LOCAL_TOOLS.split(',').any(|tool| tool == required),
                "Claude local-only policy must allow {required}"
            );
        }
        for overly_broad in [
            "Bash",
            "Bash(git *)",
            "Bash(cargo *)",
            "Bash(npm *)",
            "Bash(pnpm *)",
            "Bash(yarn *)",
        ] {
            assert!(
                !CLAUDE_LOCAL_TOOLS
                    .split(',')
                    .any(|tool| tool == overly_broad),
                "Claude local-only policy must not broadly allow {overly_broad}"
            );
        }
        for forbidden in [
            "Bash(git push)",
            "Bash(git push *)",
            "Bash(git fetch)",
            "Bash(git fetch *)",
            "Bash(git pull)",
            "Bash(git pull *)",
            "Bash(git remote)",
            "Bash(git remote *)",
            "Bash(git worktree)",
            "Bash(git worktree *)",
            "Bash(git branch)",
            "Bash(git branch *)",
            "Bash(git checkout)",
            "Bash(git checkout *)",
            "Bash(git switch)",
            "Bash(git switch *)",
            "Bash(git merge *)",
            "Bash(git rebase *)",
        ] {
            assert!(
                CLAUDE_FORBIDDEN_TOOLS
                    .split(',')
                    .any(|tool| tool == forbidden),
                "Claude local-only policy must explicitly deny {forbidden}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_executor_bridge_rejects_dot_path_dispatcher_directory() {
        // Break caught: PATH=. selecting an executable from the conductor's mutable cwd.
        use std::os::unix::fs::PermissionsExt;

        let dispatcher_name = format!(
            "autospec-relative-path-dispatcher-{}",
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let dispatcher = std::env::current_dir()
            .expect("current directory")
            .join(&dispatcher_name);
        fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write relative dispatcher");
        fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755))
            .expect("make relative dispatcher executable");
        let env = BTreeMap::from([("PATH".to_string(), OsString::from("."))]);

        let error = super::safe_executable(Path::new(&dispatcher_name), &env)
            .expect_err("relative PATH directories must be ignored");

        assert!(
            error.contains("not found on PATH"),
            "unexpected error: {error}"
        );
        let _ = fs::remove_file(dispatcher);
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_executor_bridge_rejects_empty_path_dispatcher_directory() {
        // Break caught: an empty PATH component selecting from the conductor's mutable cwd.
        use std::os::unix::fs::PermissionsExt;

        let dispatcher_name = format!(
            "autospec-empty-path-dispatcher-{}",
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let dispatcher = std::env::current_dir()
            .expect("current directory")
            .join(&dispatcher_name);
        fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write empty-path dispatcher");
        fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755))
            .expect("make empty-path dispatcher executable");
        let env = BTreeMap::from([("PATH".to_string(), OsString::from(":"))]);

        let error = super::safe_executable(Path::new(&dispatcher_name), &env)
            .expect_err("empty PATH directories must be ignored");

        assert!(
            error.contains("not found on PATH"),
            "unexpected error: {error}"
        );
        let _ = fs::remove_file(dispatcher);
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_executor_bridge_rejects_every_temporary_dispatcher_root() {
        // Break caught: the Rust bridge accepting a dispatcher that the shared shell resolver
        // rejects, especially /var/tmp and an operator-supplied TMPDIR.
        use super::temporary_path;

        let mut env = BTreeMap::new();
        env.insert(
            "TMPDIR".to_string(),
            OsString::from("/safe/operator-temporary"),
        );

        for denied in [
            "/tmp/codex",
            "/private/tmp/codex",
            "/var/tmp/codex",
            "/var/folders/session/codex",
            "/safe/operator-temporary/codex",
        ] {
            assert!(
                temporary_path(Path::new(denied), &env),
                "{denied} must be treated as temporary"
            );
        }
        for allowed in [
            "/tmp-safe/codex",
            "/private/tmp-safe/codex",
            "/var/tmp-safe/codex",
            "/var/folders-safe/codex",
            "/safe/operator-temporary-safe/codex",
        ] {
            assert!(
                !temporary_path(Path::new(allowed), &env),
                "{allowed} must not match a temporary prefix accidentally"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_executor_bridge_checks_temporary_candidate_before_canonicalization() {
        // Break caught: a symlink placed in temporary storage being accepted because its target
        // is an otherwise safe executable.
        use std::os::unix::fs::{symlink, PermissionsExt};

        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let safe_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for test fixture"))
            .join(".autospec/test-fixtures")
            .join(format!(
                "executor-safe-target-{}-{sequence}",
                std::process::id()
            ));
        fs::create_dir_all(&safe_root).expect("create safe target root");
        let safe_target = safe_root.join("codex");
        fs::write(&safe_target, "#!/bin/sh\nexit 0\n").expect("write safe target");
        fs::set_permissions(&safe_target, fs::Permissions::from_mode(0o755))
            .expect("make safe target executable");

        let var_tmp_root = PathBuf::from("/var/tmp").join(format!(
            "autospec-executor-candidate-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&var_tmp_root).expect("create /var/tmp candidate root");
        let var_tmp_link = var_tmp_root.join("codex");
        symlink(&safe_target, &var_tmp_link).expect("link temporary candidate to safe target");

        let custom_tmp = safe_root.join("operator-tmp");
        fs::create_dir_all(&custom_tmp).expect("create supplied TMPDIR");
        let custom_tmp_link = custom_tmp.join("claude");
        symlink(&safe_target, &custom_tmp_link).expect("link TMPDIR candidate to safe target");
        let env = BTreeMap::from([("TMPDIR".to_string(), custom_tmp.as_os_str().to_os_string())]);

        for candidate in [&var_tmp_link, &custom_tmp_link] {
            let error = super::safe_executable(candidate, &env)
                .expect_err("temporary candidate must fail before canonicalization");
            assert!(error.contains("temporary"), "unexpected error: {error}");
        }

        let _ = fs::remove_dir_all(var_tmp_root);
        let _ = fs::remove_dir_all(safe_root);
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_executor_bridge_checks_temporary_target_after_canonicalization() {
        // Break caught: a safe-looking configured path resolving to an executable in /var/tmp.
        use std::os::unix::fs::{symlink, PermissionsExt};

        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let safe_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for test fixture"))
            .join(".autospec/test-fixtures")
            .join(format!(
                "executor-safe-candidate-{}-{sequence}",
                std::process::id()
            ));
        fs::create_dir_all(&safe_root).expect("create safe candidate root");

        let var_tmp_root = PathBuf::from("/var/tmp").join(format!(
            "autospec-executor-target-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&var_tmp_root).expect("create /var/tmp target root");
        let temporary_target = var_tmp_root.join("codex");
        fs::write(&temporary_target, "#!/bin/sh\nexit 0\n").expect("write temporary target");
        fs::set_permissions(&temporary_target, fs::Permissions::from_mode(0o755))
            .expect("make temporary target executable");
        let var_tmp_link = safe_root.join("codex");
        symlink(&temporary_target, &var_tmp_link).expect("link safe candidate to /var/tmp target");

        let custom_tmp_root =
            PathBuf::from(std::env::var_os("HOME").expect("HOME for supplied TMPDIR fixture"))
                .join(".autospec/test-fixtures")
                .join(format!(
                    "executor-custom-tmp-target-{}-{sequence}",
                    std::process::id()
                ));
        fs::create_dir_all(&custom_tmp_root).expect("create supplied TMPDIR target root");
        let custom_tmp_target = custom_tmp_root.join("claude");
        fs::write(&custom_tmp_target, "#!/bin/sh\nexit 0\n").expect("write supplied TMPDIR target");
        fs::set_permissions(&custom_tmp_target, fs::Permissions::from_mode(0o755))
            .expect("make supplied TMPDIR target executable");
        let custom_tmp_link = safe_root.join("claude");
        symlink(&custom_tmp_target, &custom_tmp_link)
            .expect("link safe candidate to supplied TMPDIR target");

        let custom_env = BTreeMap::from([(
            "TMPDIR".to_string(),
            custom_tmp_root.as_os_str().to_os_string(),
        )]);
        let empty_env = BTreeMap::new();
        for (candidate, env) in [(&var_tmp_link, &empty_env), (&custom_tmp_link, &custom_env)] {
            let error = super::safe_executable(candidate, env)
                .expect_err("temporary canonical target must fail closed");
            assert!(error.contains("temporary"), "unexpected error: {error}");
        }

        let _ = fs::remove_dir_all(safe_root);
        let _ = fs::remove_dir_all(var_tmp_root);
        let _ = fs::remove_dir_all(custom_tmp_root);
    }

    #[test]
    fn autonomous_executor_bridge_loads_installed_config_locations() {
        // Break caught: an installed binary requiring a manual alias-table override.
        let root = test_root("installed-config");
        write_alias_table(&root, installed_aliases());
        let mut env = BTreeMap::from([
            (
                "AUTOSPEC_CONFIG_DIR".to_string(),
                root.as_os_str().to_os_string(),
            ),
            ("PATH".to_string(), OsString::from("/bin:/usr/bin")),
        ]);
        assert_eq!(
            HarnessConfig::load(&root, &env)
                .expect("load AUTOSPEC_CONFIG_DIR aliases")
                .aliases
                .len(),
            3
        );

        env.remove("AUTOSPEC_CONFIG_DIR");
        let home = test_root("installed-home");
        let standard_config = home.join(".autospec/config");
        fs::create_dir_all(&standard_config).expect("create standard config");
        write_alias_table(&standard_config, installed_aliases());
        env.insert("HOME".to_string(), home.as_os_str().to_os_string());
        assert_eq!(
            HarnessConfig::load(&root, &env)
                .expect("load standard aliases")
                .aliases
                .len(),
            3
        );
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn autonomous_executor_bridge_uses_complete_markers_and_alias_order_fallback() {
        // Break caught: CODEX_CI/OpenCode sessions and marker-free installed shells being missed.
        let root = test_root("marker-fallback");
        let table = write_alias_table(
            &root,
            "claude\tautospec-definitely-missing\t\tClaude Code\n\
             codex\tsh\t\tCodex CLI\n\
             opencode\tfalse\t\tOpenCode\n",
        );
        let mut env = environment(&table);
        env.insert("CODEX_CI".to_string(), OsString::from("1"));
        assert_eq!(
            HarnessConfig::load(&root, &env)
                .expect("load config")
                .resolve(&env)
                .expect("resolve CODEX_CI")
                .kind,
            HarnessKind::Codex
        );

        env.remove("CODEX_CI");
        env.insert(
            "OPENCODE_SESSION_ID".to_string(),
            OsString::from("session-1"),
        );
        assert_eq!(
            HarnessConfig::load(&root, &env)
                .expect("load config")
                .resolve(&env)
                .expect("resolve OpenCode session")
                .kind,
            HarnessKind::OpenCode
        );

        env.remove("OPENCODE_SESSION_ID");
        assert_eq!(
            HarnessConfig::load(&root, &env)
                .expect("load config")
                .resolve(&env)
                .expect("probe aliases in installed order")
                .kind,
            HarnessKind::Codex
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn autonomous_executor_bridge_rejects_relative_and_non_executable_dispatchers() {
        // Break caught: relative configured paths or non-executable files reaching process launch.
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("dispatcher-contract");
        let relative_table = write_alias_table(
            &root,
            "codex\t./relative/codex\t\tCodex CLI\n\
             claude\ttrue\t\tClaude Code\n\
             opencode\tfalse\t\tOpenCode\n",
        );
        let mut env = environment(&relative_table);
        env.insert("CODEX_THREAD_ID".to_string(), OsString::from("thread-1"));
        let error = HarnessConfig::load(&root, &env)
            .expect("load relative alias")
            .resolve(&env)
            .expect_err("relative dispatcher must fail closed");
        assert!(error.contains("absolute"), "unexpected error: {error}");

        let fixture_root = PathBuf::from(std::env::var_os("HOME").expect("HOME for test fixture"))
            .join(".autospec/test-fixtures")
            .join(format!(
                "non-executable-{}-{}",
                std::process::id(),
                TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&fixture_root).expect("create non-temporary fixture");
        let dispatcher = fixture_root.join("codex");
        fs::write(&dispatcher, "#!/bin/sh\nexit 0\n").expect("write dispatcher");
        fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o644))
            .expect("make dispatcher non-executable");
        write_alias_table(
            &root,
            &format!(
                "codex\t{}\t\tCodex CLI\nclaude\ttrue\t\tClaude Code\nopencode\tfalse\t\tOpenCode\n",
                dispatcher.display()
            ),
        );
        let error = HarnessConfig::load(&root, &env)
            .expect("load non-executable alias")
            .resolve(&env)
            .expect_err("non-executable dispatcher must fail closed");
        assert!(error.contains("executable"), "unexpected error: {error}");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn autonomous_executor_bridge_opencode_requires_and_uses_safe_adapter() {
        // Break caught: treating OpenCode --pure as built-in tool containment.
        let root = test_root("opencode-argv");
        let table = write_alias_table(&root, installed_aliases());
        let mut env = environment(&table);
        env.insert(
            "AUTOSPEC_HANDOFF_DISPATCHER_KIND".to_string(),
            OsString::from("opencode"),
        );
        let uncontained = HarnessConfig::load(&root, &env)
            .expect("load harness config")
            .resolve(&env)
            .expect("recognize OpenCode");

        let error = uncontained
            .invocation(
                Path::new("/safe/worktree"),
                Path::new("/safe/state/unused.txt"),
                "implement issue 42",
            )
            .expect_err("OpenCode without an adapter must fail closed");
        assert!(
            error.contains("executor_harness_uncontained"),
            "unexpected error: {error}"
        );

        env.insert(
            "AUTOSPEC_OPENCODE_CONTAINMENT_ADAPTER".to_string(),
            OsString::from("/bin/true"),
        );
        let contained = HarnessConfig::load(&root, &env)
            .expect("load contained harness config")
            .resolve(&env)
            .expect("resolve contained OpenCode");
        let invocation = contained
            .invocation(
                Path::new("/safe/worktree"),
                Path::new("/safe/state/unused.txt"),
                "implement issue 42",
            )
            .expect("build contained OpenCode invocation");

        assert_eq!(
            invocation.program,
            fs::canonicalize("/bin/true").expect("canonical adapter")
        );
        assert_eq!(
            invocation.args,
            vec![
                contained.executable.display().to_string(),
                "--pure".to_string(),
                "run".to_string(),
                "implement issue 42".to_string(),
            ]
        );
        assert!(!invocation.requires_mutation_snapshots);
        let _ = fs::remove_dir_all(root);
    }

    fn persisted_invocation() -> PersistedInvocation {
        PersistedInvocation {
            schema: 1,
            identity: BridgeIdentity {
                repository: "owner/repo".to_string(),
                issue: 42,
                worker_id: "worker-1".to_string(),
                branch: "feat/autonomous-issue-42".to_string(),
                claim_id: "claim-42".to_string(),
                invocation_id: "invocation-42".to_string(),
                base_ref: "refs/remotes/origin/main".to_string(),
                base_oid: "a".repeat(40),
                worktree: PathBuf::from("/safe/worktree"),
                runtime_session_id: Some("runtime-42".to_string()),
            },
            harness: HarnessKind::Codex,
            phase: BridgePhase::Implementing,
            process: Some(ProcessIdentity {
                pid: 123,
                process_group: 123,
                executable: PathBuf::from("/usr/bin/codex"),
                argv_digest: "b".repeat(64),
                boot_id: "boot-1".to_string(),
                start_identity: "456".to_string(),
            }),
            progress_at: 1_722_000_000,
            pr: None,
            head_oid: None,
            terminal_result: None,
        }
    }

    #[test]
    fn autonomous_executor_bridge_persisted_json_round_trips_strictly() {
        // Break caught: recovery accepting missing, mistyped, or silently defaulted identity data.
        let expected = persisted_invocation();
        let json = expected.to_json().expect("serialize invocation");
        let actual = PersistedInvocation::from_json(&json).expect("parse invocation");

        assert_eq!(actual, expected);
    }

    #[test]
    fn autonomous_executor_bridge_persisted_json_rejects_unknown_fields() {
        // Break caught: a newer or foreign state document being partially adopted.
        let expected = persisted_invocation();
        let mut value: serde_json::Value =
            serde_json::from_str(&expected.to_json().expect("serialize invocation"))
                .expect("parse test JSON");
        value
            .as_object_mut()
            .expect("invocation object")
            .insert("foreign".to_string(), serde_json::json!(true));

        let error = PersistedInvocation::from_json(&value.to_string())
            .expect_err("unknown fields must fail closed");

        assert!(
            error.contains("unexpected field"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn autonomous_executor_bridge_persisted_json_requires_supported_schema() {
        // Break caught: missing, unsupported, or wrapped schema versions being adopted.
        let expected = persisted_invocation();
        let mut value: serde_json::Value =
            serde_json::from_str(&expected.to_json().expect("serialize invocation"))
                .expect("parse test JSON");
        value
            .as_object_mut()
            .expect("invocation object")
            .remove("schema");
        let error = PersistedInvocation::from_json(&value.to_string())
            .expect_err("missing schema must fail closed");
        assert!(error.contains("missing field"), "unexpected error: {error}");

        for (unsupported, expected_message) in [
            (2_u64, "unsupported invocation schema"),
            (4_294_967_297, "out of range"),
        ] {
            let mut value: serde_json::Value =
                serde_json::from_str(&expected.to_json().expect("serialize invocation"))
                    .expect("parse test JSON");
            value
                .as_object_mut()
                .expect("invocation object")
                .insert("schema".to_string(), serde_json::json!(unsupported));
            let error = PersistedInvocation::from_json(&value.to_string())
                .expect_err("unsupported schema must fail closed");
            assert!(
                error.contains(expected_message),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn autonomous_executor_bridge_persisted_json_rejects_process_identity_overflow() {
        // Break caught: oversized process IDs wrapping onto a different live process.
        let expected = persisted_invocation();
        for field in ["pid", "process_group"] {
            let mut value: serde_json::Value =
                serde_json::from_str(&expected.to_json().expect("serialize invocation"))
                    .expect("parse test JSON");
            value
                .get_mut("process")
                .and_then(serde_json::Value::as_object_mut)
                .expect("process object")
                .insert(field.to_string(), serde_json::json!(u64::MAX));
            let error = PersistedInvocation::from_json(&value.to_string())
                .expect_err("oversized process identity must fail closed");
            assert!(error.contains("out of range"), "unexpected error: {error}");
        }
    }

    #[test]
    fn autonomous_executor_bridge_process_identity_requires_every_component() {
        // Break caught: signaling a reused PID whose executable or boot/start identity differs.
        let expected = persisted_invocation().process.expect("process identity");
        let mut observed = expected.clone();
        assert!(expected.matches(&observed));

        observed.boot_id = "other-boot".to_string();
        assert!(!expected.matches(&observed));
        observed = expected.clone();
        observed.argv_digest = "c".repeat(64);
        assert!(!expected.matches(&observed));
        observed = expected.clone();
        observed.start_identity = "457".to_string();
        assert!(!expected.matches(&observed));
    }
}
