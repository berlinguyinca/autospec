use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use autospec_core::autonomous_lifecycle::{
    decide as decide_lifecycle, decide_capacity, decide_conductor_lease, CapacityDecision,
    CapacityInput, ConductorLeaseDecision, ConductorLeaseInput, ConductorLeaseReclaim,
    LifecycleDecision, LifecycleInput, LifecycleReject, RepositoryScope,
};
use autospec_core::state::json::{JsonParser, JsonValue};

use super::CommandFailure;
pub(super) struct ResilienceStore {
    scope: RepositoryScope,
    state_root: PathBuf,
    spend_root: PathBuf,
    host: String,
}
pub(super) struct ResilienceAdmission {
    pub failure_count: u8,
    pub capacity: CapacityDecision,
    pub lease: Option<ConductorLeaseDecision>,
}
#[derive(Debug)]
pub(super) enum ResilienceReject {
    MalformedState,
    ForeignState,
    MalformedFailure,
    MalformedSpend,
}
impl ResilienceReject {
    pub(super) fn reason(&self) -> &'static str {
        match self {
            Self::MalformedState => "malformed_state",
            Self::ForeignState => "foreign_state",
            Self::MalformedFailure => "malformed_failure",
            Self::MalformedSpend => "malformed_spend",
        }
    }
}
impl ResilienceStore {
    pub(super) fn from_env(repo: &str) -> Result<Self, String> {
        let scope = RepositoryScope::try_from(repo).map_err(|reason| format!("--repo {reason}"))?;
        let state_base = env_path("AUTOSPEC_STATE_DIR", &[".autospec"]);
        let spend_root = env::var("AUTOSPEC_AUTONOMOUS_SPEND_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| state_base.join("autonomous-spend"));
        Ok(Self {
            scope,
            state_root: state_base.join("autonomous"),
            spend_root,
            host: env::var("AUTOSPEC_HOST")
                .or_else(|_| env::var("HOSTNAME"))
                .unwrap_or_default(),
        })
    }
    pub(super) fn read_state(&self) -> Result<Option<(ResilienceState, bool)>, ResilienceReject> {
        for (candidate_index, path) in self
            .candidates(&self.state_root, "state.json")
            .into_iter()
            .enumerate()
        {
            match fs::read_to_string(&path) {
                Ok(raw) => {
                    let state = ResilienceState::parse(&raw)
                        .map_err(|_| ResilienceReject::MalformedState)?;
                    if state.repo != self.scope.as_str() {
                        return Err(ResilienceReject::ForeignState);
                    }
                    return Ok(Some((state, candidate_index > 0)));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Err(ResilienceReject::MalformedState),
            }
        }
        Ok(None)
    }
    pub(super) fn read_failures(&self, issue: Option<u64>) -> Result<u8, ResilienceReject> {
        let Some(issue) = issue else {
            return Ok(0);
        };
        for path in self.candidates(&self.state_root, &format!("issues/{issue}.json")) {
            match fs::read_to_string(&path) {
                Ok(raw) => return parse_failures(&raw, issue),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Err(ResilienceReject::MalformedFailure),
            }
        }
        Ok(0)
    }
    fn read_spend(&self) -> Result<Spend, ResilienceReject> {
        for path in self.candidates(&self.spend_root, "spend.json") {
            match fs::read_to_string(&path) {
                Ok(raw) => return Spend::parse(&raw),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Err(ResilienceReject::MalformedSpend),
            }
        }
        Ok(Spend::default())
    }

    pub(super) fn write_state(&self, value: &ResilienceState) -> Result<(), String> {
        let [canonical, ..] = self.slugs();
        let path = self.state_root.join(&canonical).join("state.json");
        fs::create_dir_all(path.parent().expect("state path has a parent"))
            .map_err(|error| format!("cannot create resilience state directory: {error}"))?;
        super::atomic_write(&path, &value.to_json(&canonical))
    }

    pub(super) fn admit(
        &self,
        issue: Option<u64>,
        usage_cap: u64,
        issue_cap: u64,
    ) -> Result<ResilienceAdmission, ResilienceReject> {
        let stored_state = self.read_state()?;
        let failure_count = self.read_failures(issue)?;
        let spend = self.read_spend()?;
        let capacity = decide_capacity(CapacityInput::new(
            spend.tokens,
            usage_cap,
            spend.issues,
            issue_cap,
        ));
        if let Some((state, true)) = stored_state.as_ref() {
            self.write_state(state)
                .map_err(|_| ResilienceReject::MalformedState)?;
        }
        let lease = stored_state.and_then(|stored| {
            let state = stored.0;
            let lock_pid = state.lock_pid?;
            let known_same_host = state
                .lock_host
                .as_deref()
                .is_some_and(|host| same_known_host(host, &self.host));
            let dead_same_host_pid = known_same_host && pid_is_dead(lock_pid);
            let input = match state.heartbeat_at {
                Some(heartbeat_at) => {
                    let age = now_secs().saturating_sub(heartbeat_at);
                    if state.status == "claimed" {
                        ConductorLeaseInput::claimed(age, dead_same_host_pid)
                    } else {
                        ConductorLeaseInput::running(age, dead_same_host_pid)
                    }
                }
                None => ConductorLeaseInput::missing_heartbeat(
                    state.status == "claimed",
                    dead_same_host_pid,
                ),
            };
            Some(decide_conductor_lease(input))
        });
        Ok(ResilienceAdmission {
            failure_count,
            capacity,
            lease,
        })
    }

    fn slugs(&self) -> [String; 3] {
        let repo = self.scope.as_str();
        [
            repo.replace('/', "__"),
            repo.replace('/', "_"),
            repo.replace('/', "-"),
        ]
    }

    fn candidates(&self, root: &Path, leaf: &str) -> [PathBuf; 3] {
        let [canonical, underscore, hyphen] = self.slugs();
        [
            root.join(canonical).join(leaf),
            root.join(underscore).join(leaf),
            root.join(hyphen).join(leaf),
        ]
    }
}

pub(super) fn run(args: &[String]) -> Result<(), CommandFailure> {
    let options = DecisionOptions::parse(args).map_err(CommandFailure::diagnostic)?;
    let store = ResilienceStore::from_env(&options.repo).map_err(CommandFailure::diagnostic)?;
    let admission = match store.admit(options.issue, options.usage_cap, options.issue_cap) {
        Ok(admission) => admission,
        Err(reject) => return emit(Decision::Reject(reject.reason())),
    };
    if let Some(ConductorLeaseDecision::Held) = admission.lease {
        return emit(Decision::Held);
    }
    if matches!(
        decide_lifecycle(
            &LifecycleInput::from_scope(store.scope.clone())
                .with_failure_count(admission.failure_count),
        ),
        LifecycleDecision::Reject(LifecycleReject::FailureCap)
    ) {
        return emit(Decision::Reject("failure_cap"));
    }
    match admission.capacity {
        CapacityDecision::UsageCap => emit(Decision::Park("usage_cap")),
        CapacityDecision::IssueCap => emit(Decision::Park("issue_cap")),
        CapacityDecision::WithinCap => match admission.lease {
            Some(ConductorLeaseDecision::Reclaim(reason)) => emit(Decision::Reclaim(reason)),
            Some(ConductorLeaseDecision::Held) => emit(Decision::Held),
            None => emit(Decision::Available),
        },
    }
}

struct DecisionOptions {
    repo: String,
    issue: Option<u64>,
    usage_cap: u64,
    issue_cap: u64,
}

impl DecisionOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        if args.first().map(String::as_str) != Some("decide") {
            return Err("autonomous resilience requires the decide action".to_string());
        }
        let mut repo = None;
        let mut issue = None;
        let mut usage_cap = None;
        let mut issue_cap = None;
        let mut index = 1;
        while index < args.len() {
            let flag = &args[index];
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| format!("{flag} requires a value"))?;
            match flag.as_str() {
                "--repo" => set_once(&mut repo, value, "--repo")?,
                "--issue" => set_number(&mut issue, value, "--issue", true)?,
                "--budget-tokens" => set_number(&mut usage_cap, value, "--budget-tokens", false)?,
                "--budget-issues" => set_number(&mut issue_cap, value, "--budget-issues", false)?,
                _ => return Err(format!("unknown autonomous resilience option: {flag}")),
            }
            index += 1;
        }
        Ok(Self {
            repo: repo.ok_or_else(|| "autonomous resilience requires --repo".to_string())?,
            issue,
            usage_cap: match usage_cap {
                Some(value) => value,
                None => env_budget("AUTOSPEC_AUTONOMOUS_LIFETIME_TOKENS")?,
            },
            issue_cap: match issue_cap {
                Some(value) => value,
                None => env_budget("AUTOSPEC_AUTONOMOUS_LIFETIME_ISSUES")?,
            },
        })
    }
}

fn set_once(slot: &mut Option<String>, value: &str, flag: &str) -> Result<(), String> {
    if slot.replace(value.to_string()).is_some() {
        Err(format!("{flag} may be specified only once"))
    } else {
        Ok(())
    }
}

fn set_number(
    slot: &mut Option<u64>,
    value: &str,
    flag: &str,
    positive: bool,
) -> Result<(), String> {
    let number = value.parse::<u64>().map_err(|_| {
        format!(
            "{flag} must be a {} integer",
            if positive { "positive" } else { "non-negative" }
        )
    })?;
    if positive && number == 0 {
        return Err(format!("{flag} must be a positive integer"));
    }
    if slot.replace(number).is_some() {
        Err(format!("{flag} may be specified only once"))
    } else {
        Ok(())
    }
}

enum Decision {
    Available,
    Held,
    Reclaim(ConductorLeaseReclaim),
    Park(&'static str),
    Reject(&'static str),
}

fn emit(decision: Decision) -> Result<(), CommandFailure> {
    let (body, exit_code) = match decision {
        Decision::Available => ("{\"decision\":\"available\"}".to_string(), 0),
        Decision::Held => ("{\"decision\":\"held\"}".to_string(), 20),
        Decision::Reclaim(reason) => (
            format!(
                "{{\"decision\":\"reclaim\",\"reason\":\"{}\"}}",
                reclaim_name(reason)
            ),
            0,
        ),
        Decision::Park(reason) => (
            format!("{{\"decision\":\"park\",\"reason\":\"{reason}\"}}"),
            20,
        ),
        Decision::Reject(reason) => (
            format!("{{\"decision\":\"reject\",\"reason\":\"{reason}\"}}"),
            3,
        ),
    };
    println!("{body}");
    if exit_code == 0 {
        Ok(())
    } else {
        Err(CommandFailure::status(String::new(), exit_code))
    }
}

fn reclaim_name(reason: ConductorLeaseReclaim) -> &'static str {
    match reason {
        ConductorLeaseReclaim::DeadSameHostPid => "dead_same_host_pid",
        ConductorLeaseReclaim::MissingHeartbeat => "missing_heartbeat",
        ConductorLeaseReclaim::Abandoned => "abandoned",
        ConductorLeaseReclaim::ClaimedExpired => "claimed_expired",
    }
}

#[derive(Default)]
struct Spend {
    tokens: u64,
    issues: u64,
}

impl Spend {
    fn parse(raw: &str) -> Result<Self, ResilienceReject> {
        let mut fields = parse_json_object(raw).map_err(|_| ResilienceReject::MalformedSpend)?;
        let schema = number_field(&mut fields, "schema")
            .map_err(|_| ResilienceReject::MalformedSpend)?
            .ok_or(ResilienceReject::MalformedSpend)?;
        if schema != 1 {
            return Err(ResilienceReject::MalformedSpend);
        }
        Ok(Self {
            tokens: number_field(&mut fields, "tokens")
                .map_err(|_| ResilienceReject::MalformedSpend)?
                .ok_or(ResilienceReject::MalformedSpend)?,
            issues: number_field(&mut fields, "issues")
                .map_err(|_| ResilienceReject::MalformedSpend)?
                .ok_or(ResilienceReject::MalformedSpend)?,
        })
    }
}

pub(super) struct ResilienceState {
    repo: String,
    status: String,
    host: Option<String>,
    session: Option<String>,
    heartbeat_at: Option<u64>,
    lock_pid: Option<u32>,
    lock_host: Option<String>,
    lock_session: Option<String>,
    lock_acquired_at: Option<u64>,
}

impl ResilienceState {
    fn parse(raw: &str) -> Result<Self, ()> {
        let mut fields = parse_json_object(raw)?;
        let repo = string_field(&mut fields, "repo")?.ok_or(())?;
        let status = string_field(&mut fields, "status")?.ok_or(())?;
        let host = string_field(&mut fields, "host")?;
        let session = string_field(&mut fields, "session")?;
        let heartbeat_at = number_field(&mut fields, "heartbeat_at")?;
        let lock_pid = number_field(&mut fields, "lock_pid")?
            .map(|pid| u32::try_from(pid).map_err(|_| ()))
            .transpose()?;
        let lock_host = string_field(&mut fields, "lock_host")?;
        let lock_session = string_field(&mut fields, "lock_session")?;
        let lock_acquired_at = number_field(&mut fields, "lock_acquired_at")?;
        Ok(Self {
            repo,
            status,
            host,
            session,
            heartbeat_at,
            lock_pid,
            lock_host,
            lock_session,
            lock_acquired_at,
        })
    }

    fn to_json(&self, slug: &str) -> String {
        format!(
            "{{\"repo\":\"{}\",\"slug\":\"{}\",\"status\":\"{}\",\"host\":{},\"session\":{},\"heartbeat_at\":{},\"lock_pid\":{},\"lock_host\":{},\"lock_session\":{},\"lock_acquired_at\":{}}}\n",
            super::json_escape(&self.repo),
            super::json_escape(slug),
            super::json_escape(&self.status),
            optional_json_string(self.host.as_deref()),
            optional_json_string(self.session.as_deref()),
            optional_number(self.heartbeat_at),
            optional_number(self.lock_pid.map(u64::from)),
            optional_json_string(self.lock_host.as_deref()),
            optional_json_string(self.lock_session.as_deref()),
            optional_number(self.lock_acquired_at),
        )
    }
}

fn parse_failures(raw: &str, issue: u64) -> Result<u8, ResilienceReject> {
    let mut fields = parse_json_object(raw).map_err(|_| ResilienceReject::MalformedFailure)?;
    let recorded_issue = failure_issue_field(&mut fields)?;
    if recorded_issue != issue {
        return Err(ResilienceReject::MalformedFailure);
    }
    let failures = number_field(&mut fields, "failures")
        .map_err(|_| ResilienceReject::MalformedFailure)?
        .ok_or(ResilienceReject::MalformedFailure)?;
    Ok(u8::try_from(failures).unwrap_or(u8::MAX))
}

fn failure_issue_field(fields: &mut BTreeMap<String, JsonValue>) -> Result<u64, ResilienceReject> {
    let value = fields
        .remove("issue")
        .ok_or(ResilienceReject::MalformedFailure)?;
    let issue = match value {
        JsonValue::Number(value) => value
            .parse::<u64>()
            .map_err(|_| ResilienceReject::MalformedFailure)?,
        JsonValue::String(value) => {
            parse_positive_decimal(&value).ok_or(ResilienceReject::MalformedFailure)?
        }
        _ => return Err(ResilienceReject::MalformedFailure),
    };
    if issue == 0 {
        return Err(ResilienceReject::MalformedFailure);
    }
    Ok(issue)
}

fn parse_positive_decimal(value: &str) -> Option<u64> {
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|character| character.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn string_field(fields: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<Option<String>, ()> {
    match fields.remove(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value.into_string(key).map(Some).map_err(|_| ()),
    }
}

fn number_field(fields: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<Option<u64>, ()> {
    match fields.remove(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value.into_number(key).map(Some).map_err(|_| ()),
    }
}

fn parse_json_object(input: &str) -> Result<BTreeMap<String, JsonValue>, ()> {
    JsonParser::new(input)
        .parse()
        .and_then(|value| value.into_object("resilience record"))
        .map_err(|_| ())
}

fn env_path(name: &str, suffix: &[&str]) -> PathBuf {
    env::var(name).map(PathBuf::from).unwrap_or_else(|_| {
        let mut path = env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        for component in suffix {
            path.push(component);
        }
        path
    })
}

fn env_budget(name: &str) -> Result<u64, String> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| format!("{name} must be a non-negative integer")),
        Err(env::VarError::NotPresent) => Ok(0),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must be a non-negative integer")),
    }
}

fn same_known_host(recorded: &str, current: &str) -> bool {
    let recorded = recorded.trim();
    let current = current.trim();
    !recorded.is_empty()
        && !current.is_empty()
        && !recorded.eq_ignore_ascii_case("unknown")
        && !current.eq_ignore_ascii_case("unknown")
        && recorded == current
}

fn optional_number(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn optional_json_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", super::json_escape(value)))
        .unwrap_or_else(|| "null".to_string())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(unix)]
fn pid_is_dead(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return true;
    }
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // SAFETY: POSIX `kill(pid, 0)` only probes process existence; it does not send a signal.
    let result = unsafe { kill(pid as i32, 0) };
    result != 0 && io::Error::last_os_error().raw_os_error() != Some(1)
}

#[cfg(not(unix))]
fn pid_is_dead(_pid: u32) -> bool {
    false
}
