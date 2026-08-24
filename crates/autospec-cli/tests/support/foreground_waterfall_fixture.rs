use autospec_core::autonomous::no_work::NoWorkTier;
use autospec_core::autonomous::waterfall::{TierReceipt, WaterfallState};
use autospec_core::coordination::{ConductorPhase, ConductorState};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const REPO: &str = "test/repo";

pub struct ForegroundWaterfallFixture {
    root: PathBuf,
    repo: PathBuf,
    bin: PathBuf,
    operator: PathBuf,
    calls: PathBuf,
    mode: PathBuf,
    auto_calls: PathBuf,
    accountability: PathBuf,
    safety_reviews: PathBuf,
    executor: PathBuf,
    shell: PathBuf,
}

pub struct ForegroundRuns(Vec<Output>);

impl ForegroundRuns {
    pub fn assert_success(&self) {
        for output in &self.0 {
            assert!(
                output.status.success(),
                "foreground failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

impl ForegroundWaterfallFixture {
    pub fn empty_repository() -> Self {
        Self::new("empty")
    }

    pub fn with_clear_tier15_candidate() -> Self {
        Self::new("candidate")
    }

    pub fn with_tier15_page_failure() -> Self {
        Self::new("page-failure")
    }

    pub fn retained_at_tier2_with_ready_issue() -> Self {
        let fixture = Self::new("empty");
        fixture.run_times(2).assert_success();
        fs::write(&fixture.mode, "ready\n").expect("switch to ready queue");
        fs::write(&fixture.calls, "").expect("clear calls");
        fs::write(&fixture.auto_calls, "0\n").expect("reset ready page count");
        fixture
    }

    pub fn retained_at_tier2_with_reached_worker_cap() -> Self {
        let fixture = Self::new("empty");
        fixture.run_times(2).assert_success();
        fs::write(&fixture.mode, "active-cap\n").expect("switch to capped queue");
        fs::write(&fixture.calls, "").expect("clear calls");
        fixture
    }

    fn new(mode: &str) -> Self {
        let root = temp_dir();
        let repo = root.join("repo");
        let bin = root.join("bin");
        fs::create_dir_all(&repo).expect("create repo");
        fs::create_dir_all(&bin).expect("create bin");
        let fixture = Self {
            operator: root.join("operator"),
            calls: root.join("gh.log"),
            mode: root.join("mode"),
            auto_calls: root.join("auto-calls"),
            accountability: root.join("accountability.md"),
            safety_reviews: root.join("safety-reviews"),
            executor: root.join("executor-launched"),
            shell: root.join("shell-launched"),
            root,
            repo,
            bin,
        };
        fs::write(&fixture.mode, format!("{mode}\n")).expect("write mode");
        fs::write(&fixture.auto_calls, "0\n").expect("write counter");
        fs::write(&fixture.calls, "").expect("write calls");
        fixture.install_fakes();
        fixture
    }

    fn install_fakes(&self) {
        write_executable(&self.bin.join("gh"), FAKE_GH);
        let marker = format!(
            "#!/bin/sh\nprintf 'launched\\n' >> '{}'\nexit 99\n",
            self.executor.display()
        );
        for program in ["codex", "omx"] {
            write_executable(&self.bin.join(program), &marker);
        }
        let shell_marker = format!(
            "#!/bin/sh\nprintf launched > '{}'\nexit 99\n",
            self.shell.display()
        );
        for program in ["sh", "bash"] {
            write_executable(&self.bin.join(program), &shell_marker);
        }
    }

    pub fn run_foreground_once(&self) -> ForegroundRuns {
        self.run_times(1)
    }

    pub fn run_foreground_three_times(&self) -> ForegroundRuns {
        self.run_times(3)
    }

    pub fn run_until_tier15(&self) -> ForegroundRuns {
        self.run_times(2)
    }

    fn run_times(&self, count: usize) -> ForegroundRuns {
        ForegroundRuns(
            (0..count)
                .map(|_| self.command().output().expect("run foreground"))
                .collect(),
        )
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autospec"));
        command
            .current_dir(&self.repo)
            .args([
                "autonomous",
                "run-foreground",
                "--repo",
                REPO,
                "--repo-dir",
                self.repo.to_str().expect("repo path"),
                "--branch",
                "main",
            ])
            .env("PATH", path_with(&self.bin))
            .env("AUTOSPEC_HANDOFF_DISPATCHER_KIND", "codex")
            .env("FW_MODE", &self.mode)
            .env("FW_CALLS", &self.calls)
            .env("FW_AUTO_CALLS", &self.auto_calls)
            .env("FW_SAFETY_REVIEWS", &self.safety_reviews)
            .env("AUTOSPEC_FOREGROUND_ACCOUNTABILITY", &self.accountability)
            .env(
                "AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/support/foreground_accountability_gh.sh"
                ),
            )
            .env("AUTOSPEC_AUTONOMOUS_OPERATOR_DIR", &self.operator)
            .env("AUTOSPEC_STATE_DIR", self.root.join("state"))
            .env("AUTOSPEC_AUTONOMOUS_SPEND_DIR", self.root.join("spend"))
            .env("AUTOSPEC_AUTONOMOUS_STATE_DIR", self.root.join("health"))
            .env("AUTOSPEC_HEARTBEAT_DIR", self.root.join("heartbeats"))
            .env_remove("AUTOSPEC_CONFIG_FILE");
        if fs::read_to_string(&self.mode).is_ok_and(|mode| mode.trim() == "active-cap") {
            command.env("AUTOSPEC_MAX_CONCURRENT_REPO_WORKERS", "1");
        }
        command
    }

    pub fn cursor(&self) -> NoWorkTier {
        self.waterfall_state().current_tier()
    }

    pub fn receipt_status(&self, tier: NoWorkTier) -> String {
        let state = self.waterfall_state();
        let source = fs::read_to_string(self.waterfall_root().join(format!(
            "waterfall/{}/{}.json",
            state.next_pass_id(),
            tier.as_str()
        )))
        .expect("read tier receipt");
        TierReceipt::parse_json(&source, REPO, state.next_pass_id(), tier)
            .expect("parse receipt")
            .status()
            .as_str()
            .to_string()
    }

    pub fn tier_directory_exists(&self, tier: NoWorkTier) -> bool {
        let state = self.waterfall_state();
        self.waterfall_root()
            .join(format!(
                "waterfall/{}/{}",
                state.next_pass_id(),
                tier.as_str()
            ))
            .exists()
    }

    pub fn claim_mutations(&self) -> usize {
        let calls = fs::read_to_string(&self.calls).expect("read calls");
        calls.matches("issue edit").count()
            + calls.matches("issue comment").count()
            + calls.matches("label create").count()
            + calls.matches("--method PATCH").count()
            + calls.matches("--method POST").count()
    }

    pub fn executor_launches(&self) -> usize {
        fs::read_to_string(&self.executor)
            .unwrap_or_default()
            .lines()
            .count()
    }

    pub fn why_no_work_exists(&self) -> bool {
        self.operator.join("test_repo/why-no-work.json").exists()
    }

    pub fn safety_reviews(&self) -> Vec<u64> {
        fs::read_to_string(&self.safety_reviews)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.parse().ok())
            .collect()
    }

    pub fn tier2_record_attempts(&self) -> usize {
        usize::from(self.receipt_path(NoWorkTier::Tier2).exists())
    }

    pub fn tier15_reads(&self) -> usize {
        fs::read_to_string(&self.calls)
            .expect("read calls")
            .lines()
            .filter(|line| {
                line.contains("issues?state=open&per_page=100")
                    || line.contains("issues?state=closed&per_page=100")
            })
            .count()
    }

    pub fn waterfall_snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let root = self.waterfall_root();
        let mut snapshot = BTreeMap::new();
        collect_files(&root, &root, &mut snapshot);
        snapshot
    }

    pub fn assert_no_forbidden_waterfall_side_effects(&self, allow_discovery_harness: bool) {
        let calls = fs::read_to_string(&self.calls).expect("read calls");
        for forbidden in [
            "issue edit",
            "issue comment",
            "label create",
            "--method PATCH",
            "--method POST",
        ] {
            assert!(
                !calls.contains(forbidden),
                "unexpected mutation: {forbidden}"
            );
        }
        if !allow_discovery_harness {
            assert!(!self.executor.exists(), "executor launched");
        }
        assert!(!self.shell.exists(), "shell launched");
        assert!(!self.why_no_work_exists(), "why-no-work was written");
        assert!(!self.tier_directory_exists(NoWorkTier::Tier3));
        assert!(!self.tier_directory_exists(NoWorkTier::Tier4));
        let state = fs::read_to_string(
            self.operator
                .join("test_repo/foreground-conductor-repository.json"),
        )
        .expect("read conductor state");
        assert_eq!(
            ConductorState::parse_json(&state)
                .expect("parse conductor state")
                .phase(),
            ConductorPhase::Scan
        );
    }

    fn waterfall_root(&self) -> PathBuf {
        self.operator.join("test_repo/waterfall")
    }

    fn waterfall_state(&self) -> WaterfallState {
        let source = fs::read_to_string(self.waterfall_root().join("waterfall-state.json"))
            .expect("read waterfall state");
        WaterfallState::parse_json(&source, REPO).expect("parse waterfall state")
    }

    fn receipt_path(&self, tier: NoWorkTier) -> PathBuf {
        let state = self.waterfall_state();
        self.waterfall_root().join(format!(
            "waterfall/{}/{}.json",
            state.next_pass_id(),
            tier.as_str()
        ))
    }
}

impl Drop for ForegroundWaterfallFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn temp_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "autospec-foreground-waterfall-{}-{stamp}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).expect("create fixture root");
    path
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("write executable");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod executable");
}

fn path_with(bin: &Path) -> String {
    format!("{}:{}", bin.display(), std::env::var("PATH").expect("PATH"))
}

fn collect_files(root: &Path, current: &Path, snapshot: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let mut entries = fs::read_dir(current)
        .unwrap_or_else(|error| panic!("read {}: {error}", current.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("read entry under {}: {error}", current.display()));
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, snapshot);
        } else {
            snapshot.insert(
                path.strip_prefix(root)
                    .expect("waterfall-relative path")
                    .to_path_buf(),
                fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
            );
        }
    }
}

const FAKE_GH: &str = r####"#!/bin/sh
set -eu
if [ -n "${AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER:-}" ]; then . "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_HANDLER"; fi
printf '%s ' "$@" >> "$FW_CALLS"; printf '\n' >> "$FW_CALLS"
mode="$(cat "$FW_MODE")"
endpoint=""
for value in "$@"; do case "$value" in repos/*) endpoint="$value";; esac; done
if [ "$1" = api ] && [ "$2" = graphql ]; then printf '%s\n' '{"items":[],"page_info":{"has_next_page":false,"end_cursor":null}}'; exit 0; fi
if [ "$1" = repo ] && [ "$2" = view ]; then printf '%s\n' main; exit 0; fi
if [ "$1" = pr ] && [ "$2" = list ]; then printf '%s\n' '[]'; exit 0; fi
case "$endpoint" in
  repos/test/repo/branches/main) printf '%s\n' '{}'; exit 0;;
  repos/test/repo/commits/main/status) printf '%s\n' '{"state":"success","total_count":1,"statuses":[{"context":"ci","state":"success"}]}'; exit 0;;
  *labels=in-progress-by-bot*)
    if [ "$mode" = active-cap ]; then printf '%s\n' '{"raw_count":1,"items":[{"number":91,"title":"Active work","body":"Active implementation.","labels":["in-progress-by-bot"],"author":{"login":"agent"}}]}'; else printf '%s\n' '{"raw_count":0,"items":[]}'; fi
    exit 0;;
  *labels=auto-implement*)
    if [ "$mode" = ready ]; then
      n="$(cat "$FW_AUTO_CALLS")"; n=$((n+1)); printf '%s\n' "$n" > "$FW_AUTO_CALLS"
      if [ "$n" -eq 2 ]; then printf '%s\n' 42 >> "$FW_SAFETY_REVIEWS"; fi
      if [ "$n" -le 2 ]; then printf '%s\n' '{"raw_count":1,"items":[{"number":42,"title":"Add ready work","body":"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->","labels":["auto-implement","safety:reviewed"],"author":{"login":"agent"}}]}'; else printf '%s\n' '{"raw_count":0,"items":[]}'; fi
    else printf '%s\n' '{"raw_count":0,"items":[]}'; fi
    exit 0;;
  *issues\?state=open*)
    if [ "$mode" = page-failure ]; then printf '%s\n' 'tier15 injected read failure' >&2; exit 1; fi
    if [ "$mode" = candidate ]; then printf '%s\n' '{"raw_count":1,"items":[{"number":77,"title":"Candidate","body":"Fix: retain a clear candidate without admitting it to the ready queue.","labels":[],"author":{"login":"agent"},"state":"OPEN"}]}'; else printf '%s\n' '{"raw_count":0,"items":[]}'; fi
    exit 0;;
  *issues\?state=closed*) printf '%s\n' '{"raw_count":0,"items":[]}'; exit 0;;
  repos/test/repo/issues/91/comments) printf '%s\n' '[{"id":100,"updated_at":"2026-07-16T00:00:00Z","body":"<!-- autospec-run-state:begin -->\n{\"schema\":1,\"repo\":\"test/repo\",\"issue\":91,\"worker_id\":\"worker-91\",\"state\":\"claimed\",\"branch\":\"feat/91\",\"pr\":\"17\",\"step\":\"claimed\",\"paths\":[],\"claimed_at\":\"2026-07-16T00:00:00Z\",\"updated_at\":\"2026-07-16T00:00:00Z\",\"ttl_seconds\":10800}\n<!-- autospec-run-state:end -->"}]'; exit 0;;
esac
printf 'unexpected gh invocation: %s\n' "$*" >&2
exit 1
"####;
