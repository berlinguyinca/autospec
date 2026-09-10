//! Staged-spec freshness and completeness (#3864).
//!
//! A dispatch worker runs as its own user on a cluster node where `gh` is not
//! authenticated, so the merge host — the one host with read access — stages
//! the issue as a markdown spec and the worker is handed that file. The design
//! is sound until the worker starts an hour after the spec was written: the
//! staged copy is then a perfectly faithful copy of an issue that no longer
//! exists, and nothing in the bundle says so. The lost work is invisible
//! precisely because the copy is faithful.
//!
//! Four invariants, each a primitive here:
//!
//! 1. **A staged spec carries the revision of its source**
//!    ([`HEADER_SOURCE_UPDATED_AT`], [`staged_source_updated_at`]). Without the
//!    source issue's `updatedAt` there is no way to tell a faithful copy from a
//!    stale one, and the only repair is to re-run from scratch.
//! 2. **The discussion is part of the task**
//!    ([`IssueSnapshot::comments_since_body_edit`]). A comment added after the
//!    body's last edit is a clarification of the task, and the spec that
//!    omits it produces work that has to be redone.
//! 3. **Unverifiable freshness is a refusal, not a warning**
//!    ([`DispatchVerdict::Refuse`]). A dispatcher that cannot reach the
//!    tracker, cannot read a staged revision, or has no staged spec at all
//!    names the issue and the last known staging time instead of running the
//!    stale copy.
//! 4. **The environment is declared, not assumed** ([`EnvironmentProbe`]).
//!    `docker` is not the container runtime on these workers — `apptainer` is,
//!    and it is already in PATH. A task written against a runtime that is
//!    absent fails slowly and misleadingly, so the spec names what is present,
//!    what is absent, and what was never probed.
//!
//! A fifth concern, the same drift from the grading side (#3925): when the
//! caller knows the gate set the patch will be graded against, the staged
//! spec carries it as acceptance criteria ([`IssueSnapshot::gates`],
//! rendered from [`crate::grading`]). The spec a worker reads names the same
//! gate commands that decide whether the patch lands, so a run graded
//! against a weaker set is not one this spec asked for. A spec that names
//! no gates renders no gate section.
//!
//! Everything here is pure and testable: no I/O, no clock, no subprocess. The
//! caller fetches the issue, probes its own host, and calls
//! [`IssueSnapshot::stage`] when staging and [`authorize`] when dispatching.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::grading::Gate;

/// Header recording the source issue number.
pub const HEADER_ISSUE: &str = "# staged-spec issue:";

/// Header recording the instant the spec was staged, in epoch seconds. This is
/// the "last known staging time" a refusal has to name.
pub const HEADER_STAGED_AT: &str = "# staged-at:";

/// Header recording the source issue's `updatedAt` at staging time, in epoch
/// seconds. A staged spec without it cannot be aged, so it cannot be trusted.
pub const HEADER_SOURCE_UPDATED_AT: &str = "# source-updated-at:";

/// Header counting the comments included from the discussion.
pub const HEADER_COMMENTS: &str = "# comments-included:";

/// Section holding the host facts a worker cannot discover for itself.
pub const ENVIRONMENT_SECTION: &str = "## Execution environment";

/// Section holding the discussion that post-dates the body's last edit.
pub const DISCUSSION_SECTION: &str = "## Discussion since the last body edit";

/// Section holding the issue body verbatim — the acceptance criteria a worker
/// must be able to satisfy from the staged spec alone.
pub const BODY_SECTION: &str = "## Issue body";

/// Environment entry naming the container runtime available to the worker.
pub const KEY_CONTAINER_RUNTIME: &str = "container-runtime";

/// Environment entry naming database availability.
pub const KEY_DATABASE: &str = "database";

/// Environment entry naming registry or network reachability.
pub const KEY_REGISTRY: &str = "registry";

/// Environment entry listing what the probe looked for and did not find.
pub const KEY_ABSENT: &str = "absent";

/// The value written for a fact that was deliberately not checked. A worker
/// reading `not probed` knows to check, or to avoid depending on it.
pub const NOT_PROBED: &str = "not probed";

/// The value written for a capability that was checked and is not there.
pub const ABSENT: &str = "absent";

/// The status token for a dispatch refused because the worker was never
/// handed a task (#3620): the staged spec is missing or empty. Distinct from
/// a freshness hold — a run refused `NO-SPEC` spent no tokens on a task
/// nobody defined, so it must not read as a baseline or staleness problem.
pub const NO_SPEC_STATUS: &str = "NO-SPEC";

/// One comment on the source issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    /// Who wrote it, for attribution in the staged spec.
    pub author: String,
    /// When it was authored, in epoch seconds.
    pub created_at: u64,
    pub body: String,
}

/// The source issue as read at staging time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSnapshot {
    pub number: u64,
    pub title: String,
    /// The body verbatim.
    pub body: String,
    /// The source issue's `updatedAt`, in epoch seconds. This is the revision
    /// the staged copy claims to be of.
    pub source_updated_at: u64,
    /// The last time the *body* was edited, when the caller knows it. A
    /// comment moves `updatedAt` too, so this cannot be derived from it.
    /// `None` means unknown, and unknown includes the whole discussion — the
    /// safe direction, because dropping a clarification is what this field
    /// exists to prevent.
    #[serde(default)]
    pub body_updated_at: Option<u64>,
    /// Every comment on the source issue, in any order.
    pub comments: Vec<IssueComment>,
    /// The gate set the patch is graded against, in gate-set order
    /// (#3925). Empty when the caller does not name one: the spec then
    /// carries no gate section and [`crate::grading::staged_gates`] reads
    /// it back as absent.
    #[serde(default)]
    pub gates: Vec<Gate>,
}

impl IssueSnapshot {
    /// The comments that belong in the staged spec, oldest first.
    ///
    /// With a known body-edit instant: strictly the comments authored after
    /// it. With no known instant: all of them, because an unknown edit time
    /// cannot rule any comment out.
    pub fn comments_since_body_edit(&self) -> Vec<&IssueComment> {
        let mut kept: Vec<&IssueComment> = self
            .comments
            .iter()
            .filter(|comment| match self.body_updated_at {
                Some(edited) => comment.created_at > edited,
                None => true,
            })
            .collect();
        kept.sort_by_key(|comment| (comment.created_at, comment.author.as_str()));
        kept
    }

    /// Render the staged spec: revision headers, then the execution
    /// environment, then the discussion, then the body verbatim, and — when
    /// the snapshot names a gate set — the gate section last, so the
    /// acceptance criteria a worker satisfies and the gates that grade the
    /// result are one document (#3925).
    pub fn stage(&self, environment: &EnvironmentProbe, staged_at: u64) -> String {
        let discussion = self.comments_since_body_edit();
        let mut out = String::new();
        out.push_str(&format!("{HEADER_ISSUE} {}\n", self.number));
        out.push_str(&format!(
            "{HEADER_STAGED_AT} {} ({})\n",
            staged_at,
            format_timestamp(staged_at)
        ));
        out.push_str(&format!(
            "{HEADER_SOURCE_UPDATED_AT} {} ({})\n",
            self.source_updated_at,
            format_timestamp(self.source_updated_at)
        ));
        out.push_str(&format!(
            "{HEADER_COMMENTS} {} of {}\n",
            discussion.len(),
            self.comments.len()
        ));
        out.push('\n');
        out.push_str(&format!("# Issue #{}: {}\n", self.number, self.title));
        out.push('\n');
        out.push_str(ENVIRONMENT_SECTION);
        out.push('\n');
        for line in environment.render() {
            out.push_str(&format!("- {line}\n"));
        }
        out.push('\n');
        out.push_str(DISCUSSION_SECTION);
        out.push('\n');
        match self.body_updated_at {
            // Say so when the inclusion rule fell back to keeping everything,
            // so a reader never mistakes "all comments" for "new comments".
            None => out.push_str("_No body edit time recorded, so every comment is included._\n\n"),
            Some(at) if discussion.is_empty() => out.push_str(&format!(
                "_No comments since the last body edit at {}._\n\n",
                format_timestamp(at)
            )),
            Some(_) => {}
        }
        if !discussion.is_empty() {
            for (index, comment) in discussion.iter().enumerate() {
                out.push_str(&format!(
                    "### Comment {} — {} at {} ({})\n\n",
                    index + 1,
                    comment.author,
                    comment.created_at,
                    format_timestamp(comment.created_at)
                ));
                out.push_str(comment.body.trim_end());
                out.push_str("\n\n");
            }
        }
        out.push_str(BODY_SECTION);
        out.push('\n');
        out.push_str(self.body.trim_end());
        out.push('\n');
        if !self.gates.is_empty() {
            out.push('\n');
            out.push_str(&crate::grading::gate_section(&self.gates));
        }
        out
    }
}

/// What one environment fact resolved to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    /// Present, with the path or endpoint that proves it.
    Present { value: String },
    /// Looked for and not there.
    Absent,
    /// Deliberately not checked; the worker must not assume either way.
    NotProbed,
}

impl ProbeState {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Present { value } => value,
            Self::Absent => ABSENT,
            Self::NotProbed => NOT_PROBED,
        }
    }

    pub fn probed(&self) -> bool {
        !matches!(self, Self::NotProbed)
    }
}

/// One declared environment fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentEntry {
    pub name: String,
    pub state: ProbeState,
    /// How the state was established, so a reader can judge it
    /// (`$PATH lookup`, `not probed by design`, `reported by --database`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub how: Option<String>,
}

/// The host facts a staged spec declares on the worker's behalf.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProbe {
    pub entries: Vec<EnvironmentEntry>,
}

impl EnvironmentProbe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one fact. Later entries never replace earlier ones: the probe
    /// order is the report order, and a duplicate name is a caller bug the
    /// rendered block makes visible rather than silently resolves.
    pub fn with(mut self, name: &str, state: ProbeState, how: Option<String>) -> Self {
        self.entries.push(EnvironmentEntry {
            name: name.to_string(),
            state,
            how,
        });
        self
    }

    pub fn get(&self, name: &str) -> Option<&EnvironmentEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Render as `- name: value (how)` lines.
    pub fn render(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| match &entry.how {
                Some(how) => format!("{}: {} [{how}]", entry.name, entry.state.as_str()),
                None => format!("{}: {}", entry.name, entry.state.as_str()),
            })
            .collect()
    }
}

/// Read `header <digits>` from the header region of a staged spec.
///
/// Only the leading run of `# `-prefixed metadata lines is considered: a body
/// or comment that quotes a header line cannot forge the revision of the spec
/// that contains it.
fn header_u64(text: &str, header: &str) -> Option<u64> {
    for line in text.lines() {
        if !line.starts_with("# ") {
            break;
        }
        if let Some(rest) = line.strip_prefix(header) {
            let token = rest.trim().split_whitespace().next()?;
            return token.parse::<u64>().ok();
        }
    }
    None
}

/// The source revision a staged spec claims, `None` when it records none.
pub fn staged_source_updated_at(text: &str) -> Option<u64> {
    header_u64(text, HEADER_SOURCE_UPDATED_AT)
}

/// When the staged spec was written, `None` when the header is missing.
pub fn staged_at(text: &str) -> Option<u64> {
    header_u64(text, HEADER_STAGED_AT)
}

/// Why freshness could not be established. Every variant is a refusal: there
/// is no "assume it is fine" branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefuseReason {
    /// No staged spec on disk at the expected path.
    StagedSpecAbsent,
    /// The staged file exists but is empty or whitespace-only. A worker
    /// handed this file has no task at all: the #3620 run invented one from
    /// the branch name and exited 0 after 17 minutes of work nobody asked
    /// for. This is a refusal, never a "fill it in later" case.
    NoSpec { bytes: usize },
    /// A staged spec predating the revision header cannot be aged at all.
    NoStagedRevision,
    /// The live issue could not be read — no credentials, no network, an
    /// unparseable response.
    LiveUnknown { detail: String },
}

/// The dispatcher's verdict on the staged spec it is about to hand over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchVerdict {
    /// The staged revision equals the live revision; dispatch may proceed.
    Proceed {
        source_updated_at: u64,
        staged_at: Option<u64>,
    },
    /// The live issue moved since staging. The caller regenerates the spec —
    /// including any comments filed since — and re-checks.
    Restage {
        staged_source_updated_at: u64,
        live_updated_at: u64,
        staged_at: Option<u64>,
    },
    /// Freshness is unknown, so the staged copy might be stale, and running
    /// stale work is the failure this verdict exists to prevent.
    Refuse {
        reason: RefuseReason,
        staged_at: Option<u64>,
    },
}

impl DispatchVerdict {
    /// True when the dispatcher must not hand the staged spec to a worker.
    pub fn held(&self) -> bool {
        !matches!(self, Self::Proceed { .. })
    }

    /// True when the held dispatch is recoverable by re-staging.
    pub fn needs_restage(&self) -> bool {
        matches!(self, Self::Restage { .. })
    }

    /// The one-line verdict, shaped for dispatch logs and wrapper scripts. A
    /// refusal always names the issue and the last known staging time (#3864
    /// acceptance criterion 4).
    pub fn line(&self, issue: u64, path: &str) -> String {
        match self {
            Self::Proceed {
                source_updated_at,
                staged_at,
            } => format!(
                "STAGED-SPEC issue {issue} current: staged {} at {path} matches live updatedAt {}",
                staged_at_line(staged_at),
                format_timestamp(*source_updated_at)
            ),
            Self::Restage {
                staged_source_updated_at,
                live_updated_at,
                staged_at,
            } => format!(
                "STAGED-SPEC issue {issue} STALE: staged {} at {path} carries updatedAt {}, live issue is {}; re-stage before dispatch",
                staged_at_line(staged_at),
                format_timestamp(*staged_source_updated_at),
                format_timestamp(*live_updated_at)
            ),
            Self::Refuse { reason, staged_at } => match staged_at {
                Some(_) => format!(
                    "STAGED-SPEC issue {issue} REFUSED: {}; last known staging time {}",
                    reason_line(reason),
                    staged_at_line(staged_at)
                ),
                None => format!(
                    "STAGED-SPEC issue {issue} REFUSED: {}; {}",
                    reason_line(reason),
                    staged_at_line(staged_at)
                ),
            },
        }
    }

    pub fn to_json(&self, issue: u64, path: &str) -> String {
        #[derive(Serialize)]
        struct Report<'a> {
            issue: u64,
            path: &'a str,
            verdict: &'a DispatchVerdict,
            held: bool,
            needs_restage: bool,
            line: String,
        }
        serde_json::to_string_pretty(&Report {
            issue,
            path,
            verdict: self,
            held: self.held(),
            needs_restage: self.needs_restage(),
            line: self.line(issue, path),
        })
        .unwrap_or_else(|_| "{}".to_string())
    }
}

fn staged_at_line(staged_at: &Option<u64>) -> String {
    match staged_at {
        Some(at) => format!("at {at} ({})", format_timestamp(*at)),
        None => "no staging time recorded".to_string(),
    }
}

fn reason_line(reason: &RefuseReason) -> String {
    match reason {
        RefuseReason::StagedSpecAbsent => format!(
            "{NO_SPEC_STATUS}: no staged spec on disk, and nothing was staged to compare against"
        ),
        RefuseReason::NoSpec { bytes } => format!(
            "{NO_SPEC_STATUS}: the staged spec is empty ({bytes} bytes); a worker handed an empty spec invents its own task, so nothing is dispatched"
        ),
        RefuseReason::NoStagedRevision => {
            "the staged spec records no source updatedAt, so it cannot be aged"
                .to_string()
        }
        RefuseReason::LiveUnknown { detail } => format!(
            "the live issue cannot be read ({detail}); a dispatcher that cannot verify freshness does not run a copy it cannot age"
        ),
    }
}

/// Decide whether the staged spec may be handed to a worker.
///
/// `staged` is the staged spec text (`None` when there is no file), `live` is
/// the source issue's current `updatedAt` (`None` when it could not be read).
///
/// The ordering is the fail-closed one: a missing spec is refused before the
/// live read is even asked for, an un-ageable spec is refused rather than
/// treated as fresh, and an unreadable live revision is a refusal — never a
/// default of "unchanged".
pub fn authorize(staged: Option<&str>, live: Option<u64>) -> DispatchVerdict {
    let Some(text) = staged else {
        return DispatchVerdict::Refuse {
            reason: RefuseReason::StagedSpecAbsent,
            staged_at: None,
        };
    };
    // The checked read's fail-closed half: a file that passes an existence
    // check but carries no text is `NO-SPEC` (#3620), refused before the
    // revision is even parsed — an empty spec is not an un-ageable one, it
    // is no task at all.
    if spec_is_empty(text) {
        return DispatchVerdict::Refuse {
            reason: RefuseReason::NoSpec { bytes: text.len() },
            staged_at: None,
        };
    }
    let staged_at = staged_at(text);
    let Some(staged_revision) = staged_source_updated_at(text) else {
        return DispatchVerdict::Refuse {
            reason: RefuseReason::NoStagedRevision,
            staged_at,
        };
    };
    let Some(live_revision) = live else {
        return DispatchVerdict::Refuse {
            reason: RefuseReason::LiveUnknown {
                detail: "the source issue's updatedAt could not be read".to_string(),
            },
            staged_at,
        };
    };
    if staged_revision == live_revision {
        DispatchVerdict::Proceed {
            source_updated_at: staged_revision,
            staged_at,
        }
    } else {
        DispatchVerdict::Restage {
            staged_source_updated_at: staged_revision,
            live_updated_at: live_revision,
            staged_at,
        }
    }
}

/// True when the staged spec text carries no task: empty or whitespace only.
/// A file that passes a bare existence check but fails this one is the
/// `NO-SPEC` case (#3620) — the mechanism to catch it is a checked read
/// (`[ -s "$f" ]` in shell, this predicate here), never a silently-swallowed
/// failed read.
pub fn spec_is_empty(text: &str) -> bool {
    text.trim().is_empty()
}

/// The receipt a run records in `status.txt` for the spec it actually saw
/// (#3620): the byte count and the sha256, so "which spec did this run
/// actually see" is answerable after the fact, not argued over from memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecReceipt {
    /// The spec's size in bytes.
    pub bytes: usize,
    /// The spec's sha256, lowercase hex.
    pub sha256: String,
}

impl SpecReceipt {
    /// The receipt of one spec's text.
    pub fn of(text: &str) -> Self {
        Self {
            bytes: text.len(),
            sha256: sha256_hex(text.as_bytes()),
        }
    }

    /// Render as the `spec-bytes=… spec-sha256=…` tokens a run appends to
    /// its `status.txt`.
    pub fn line(&self) -> String {
        format!("spec-bytes={} spec-sha256={}", self.bytes, self.sha256)
    }
}

/// The verdict on a dispatch prompt before a single token is spent (#3620).
/// A prompt whose issue section arrived empty is a programming error — the
/// #3620 run's prompt was exactly `===== ISSUE #15 ===== / ===== END ISSUE
/// =====` with nothing between — and it must be caught before dispatch, not
/// discovered from the agent's first line of plausible work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptVerdict {
    /// The prompt carries the staged spec's text.
    CarriesSpec,
    /// The prompt is empty: the caller assembled nothing to send.
    EmptyPrompt,
    /// The prompt is non-empty but the spec's text is not in it — the issue
    /// section is blank and the agent would be working from nothing.
    SpecMissingFromPrompt,
}

impl PromptVerdict {
    /// True when the prompt may be handed to an agent.
    pub fn ok(&self) -> bool {
        matches!(self, Self::CarriesSpec)
    }

    /// The one-line verdict for dispatch logs. Both failures carry the
    /// `NO-SPEC` token: the run that would have started is a no-spec run.
    pub fn line(&self, issue: u64) -> String {
        match self {
            Self::CarriesSpec => format!(
                "PROMPT issue {issue} OK: the prompt carries the staged spec text"
            ),
            Self::EmptyPrompt => format!(
                "PROMPT issue {issue} REFUSED: {NO_SPEC_STATUS} — the dispatch prompt is empty; nothing was assembled to send"
            ),
            Self::SpecMissingFromPrompt => format!(
                "PROMPT issue {issue} REFUSED: {NO_SPEC_STATUS} — the prompt carries no issue text; a prompt with no task is a programming error"
            ),
        }
    }
}

/// Assert the prompt contains the issue text before dispatch. `spec` is the
/// staged spec text the caller embeds between the prompt's issue markers.
/// One containment test catches the empty-issue-section shape: a `cat` that
/// failed into an unchecked command substitution leaves `$BODY` empty, and
/// this is the check that notices before any token is spent.
pub fn prompt_verdict(prompt: &str, spec: &str) -> PromptVerdict {
    if spec_is_empty(prompt) {
        return PromptVerdict::EmptyPrompt;
    }
    if !spec_is_empty(spec) && prompt.contains(spec.trim_end()) {
        PromptVerdict::CarriesSpec
    } else {
        PromptVerdict::SpecMissingFromPrompt
    }
}

/// Lowercase hex sha256 of one buffer.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The GitHub REST endpoint for one issue, `None` when `repo` is not a plain
/// `owner/name` pair.
///
/// The endpoint is built from operator-supplied text and handed to `gh api`,
/// so the segments are restricted the way the trusted-git refetch restricts
/// them: no traversal, no empty segment, no stray path after the name.
pub fn issue_endpoint(repo: &str, issue: u64) -> Option<String> {
    let mut segments = repo.trim().split('/');
    let owner = segments.next()?;
    let name = segments.next()?;
    if segments.next().is_some() || !valid_segment(owner) || !valid_segment(name) {
        return None;
    }
    Some(format!("repos/{owner}/{name}/issues/{issue}"))
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Parse an instant: epoch seconds, or an RFC 3339 / GitHub-style
/// `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` timestamp.
pub fn parse_timestamp(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(secs) = text.parse::<u64>() {
        return Some(secs);
    }
    parse_rfc3339(text)
}

/// Render an instant the way GitHub does: `YYYY-MM-DDTHH:MM:SSZ`.
pub fn format_timestamp(secs: u64) -> String {
    let (year, month, day) = civil_from_days(secs as i64 / 86_400);
    let tod = secs.rem_euclid(86_400);
    let (hour, minute, second) = (tod / 3_600, (tod % 3_600) / 60, tod % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// RFC 3339 in the shapes GitHub emits (`Z`) and the offset form, with an
/// optional fractional second. Deliberately strict: a response that does not
/// parse is reported as unreadable, which is a refusal, not a zero.
fn parse_rfc3339(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !(bytes[10] == b'T' || bytes[10] == b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let number =
        |range: std::ops::Range<usize>| -> Option<i64> { text.get(range)?.parse::<i64>().ok() };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut index = 19;
    // Optional fractional seconds: `.`, then at least one digit.
    if bytes.get(index) == Some(&b'.') {
        let start = index + 1;
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == start {
            return None;
        }
        index = end;
    }
    let offset = match bytes.get(index) {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(b'+') | Some(b'-') => {
            if bytes.len() < index + 6 || bytes[index + 3] != b':' {
                return None;
            }
            let sign = if bytes[index] == b'-' { -1 } else { 1 };
            let offset_hours = number(index + 1..index + 3)?;
            let offset_minutes = number(index + 4..index + 6)?;
            sign * (offset_hours * 3_600 + offset_minutes * 60)
        }
        Some(_) => return None,
    };
    let days = days_from_civil(year, month, day)?;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second - offset;
    u64::try_from(seconds).ok()
}

/// Days since the unix epoch for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`). Returns `None` for an out-of-range day-of-month rather
/// than rolling it into the next month.
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || day < 1 {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

/// The civil date for days since the unix epoch; the inverse of
/// [`days_from_civil`], used to render a recorded epoch back as an instant.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}
