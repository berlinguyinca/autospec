//! Rule 2 — a patch names the base it was verified against (issue #3708).
//!
//! With the base recorded, the converter distinguishes "stale but clean" from
//! "conflicts in its own subject matter" without applying anything, and an
//! emit that carries no base is a defect in the emit rather than a missing
//! fact about the patch ([`MetaViolation`]).

use crate::rebaseline::{BaseDrift, MeasureError};

/// Why a patch's own metadata reports an unusable emit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaViolation {
    /// The patch was finalised with the trunk ahead and no re-baseline: the
    /// work is verifiably stale, and every verdict on it was measured
    /// against a tree it will not land on.
    EmittedStale { commits_behind: u64 },
    /// The emit claims a re-baseline but carries no gate receipt: the
    /// re-baseline's `rerun_gates` step produced no evidence, so the claim is
    /// unverified.
    RebaselineWithoutGates,
}

impl MetaViolation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::EmittedStale { .. } => "emitted_stale",
            Self::RebaselineWithoutGates => "rebaseline_without_gates",
        }
    }
}

/// The metadata a run emits alongside its patch.
///
/// The base SHA is the field that makes the rest of the pipeline cheap: with
/// it, the converter knows how far behind a patch is without applying it, and
/// knows a clean apply on a stale base is staleness rather than conflict.
/// Without it, every one of those questions costs a GPU run to re-ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchMeta {
    /// The patch file or patch id the metadata describes.
    pub patch: String,
    /// The commit the work was based on, and the commit its gates last ran
    /// green against.
    pub base_sha: String,
    /// The trunk tip when the patch was finalised.
    pub tip_sha: String,
    /// `base_sha`'s distance behind `tip_sha` at emit time.
    pub commits_behind: u64,
    /// Whether the run rebased onto `tip_sha` before emitting.
    pub rebaselined: bool,
    /// Gate stages re-run green against `base_sha` (e.g. `check`, `test`).
    pub gates: Vec<String>,
}

impl PatchMeta {
    /// Builds the metadata and cross-checks it, so an emit that is internally
    /// inconsistent is refused at write time rather than discovered at
    /// conversion time.
    pub fn new(
        patch: &str,
        base_sha: &str,
        tip_sha: &str,
        commits_behind: u64,
        rebaselined: bool,
        gates: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, MeasureError> {
        let drift = BaseDrift::measure(base_sha, tip_sha, commits_behind)?;
        if patch.trim().is_empty() {
            return Err("patch id is empty");
        }
        Ok(Self {
            patch: patch.trim().to_string(),
            base_sha: drift.base_sha,
            tip_sha: drift.tip_sha,
            commits_behind: drift.commits_behind,
            rebaselined,
            gates: gates
                .into_iter()
                .map(|g| g.as_ref().trim().to_string())
                .filter(|g| !g.is_empty())
                .collect(),
        })
    }

    /// The emit-time integrity check: what the metadata says about itself.
    pub fn integrity(&self) -> Option<MetaViolation> {
        if self.commits_behind > 0 && !self.rebaselined {
            return Some(MetaViolation::EmittedStale {
                commits_behind: self.commits_behind,
            });
        }
        if self.rebaselined && self.gates.is_empty() {
            return Some(MetaViolation::RebaselineWithoutGates);
        }
        None
    }

    /// Measures the recorded base against the tip the patch is now being
    /// considered against, which need not be the tip at emit time.
    pub fn drift_against(&self, current_tip: &str) -> Result<BaseDrift, MeasureError> {
        BaseDrift::measure(&self.base_sha, current_tip, self.commits_behind)
    }

    /// Renders the sidecar: `key=value` tokens, one line.
    pub fn render(&self) -> String {
        format!(
            "patch={} base_sha={} tip_sha={} commits_behind={} rebaselined={} gates={}",
            self.patch,
            self.base_sha,
            self.tip_sha,
            self.commits_behind,
            if self.rebaselined { "1" } else { "0" },
            if self.gates.is_empty() {
                "-".to_string()
            } else {
                self.gates.join(",")
            },
        )
    }
}

/// Parses a sidecar rendered by [`PatchMeta::render`].
///
/// A missing `base_sha` is an error, not a default: a patch whose base is
/// unknown is a patch whose staleness nobody can compute, and the pipeline's
/// failure mode with such a patch is to re-test it forever.
pub fn parse_patch_meta(text: &str) -> Result<PatchMeta, String> {
    let mut patch = None;
    let mut base_sha = None;
    let mut tip_sha = None;
    let mut commits_behind = None;
    let mut rebaselined = None;
    let mut gates: Option<Vec<String>> = None;

    for token in text.split_whitespace() {
        let (key, value) = token
            .split_once('=')
            .ok_or_else(|| format!("patch meta token is not key=value: {token}"))?;
        match key {
            "patch" => patch = Some(value.to_string()),
            "base_sha" => base_sha = Some(value.to_string()),
            "tip_sha" => tip_sha = Some(value.to_string()),
            "commits_behind" => {
                commits_behind =
                    Some(value.parse::<u64>().map_err(|_| {
                        format!("patch meta commits_behind is not an integer: {value}")
                    })?)
            }
            "rebaselined" => rebaselined = Some(parse_bool(key, value)?),
            "gates" => {
                gates = Some(
                    value
                        .split(',')
                        .filter(|g| *g != "-" && !g.is_empty())
                        .map(str::to_string)
                        .collect(),
                )
            }
            other => return Err(format!("unknown patch meta key: {other}")),
        }
    }

    let gates = gates.unwrap_or_default();
    PatchMeta::new(
        patch.as_deref().ok_or("patch meta has no patch")?,
        base_sha.as_deref().ok_or("patch meta has no base_sha")?,
        tip_sha.as_deref().ok_or("patch meta has no tip_sha")?,
        commits_behind.ok_or("patch meta has no commits_behind")?,
        rebaselined.ok_or("patch meta has no rebaselined")?,
        gates,
    )
    .map_err(|err| err.to_string())
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        other => Err(format!("patch meta {key} is not a boolean: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rebaseline::support::{BASE, TIP};

    fn meta(behind: u64, rebaselined: bool, gates: &[&str]) -> PatchMeta {
        let (tip, gates_owned): (&str, Vec<String>) = (
            if behind == 0 { BASE } else { TIP },
            gates.iter().map(|g| g.to_string()).collect(),
        );
        PatchMeta::new(
            "out/issue-7/patch.diff",
            BASE,
            tip,
            behind,
            rebaselined,
            gates_owned,
        )
        .unwrap()
    }

    #[test]
    fn a_stale_emit_is_a_violation() {
        assert_eq!(
            meta(4, false, &[]).integrity(),
            Some(MetaViolation::EmittedStale { commits_behind: 4 })
        );
        assert_eq!(meta(0, false, &[]).integrity(), None);
    }

    #[test]
    fn a_rebaseline_without_gate_receipts_is_a_violation() {
        assert_eq!(
            meta(4, true, &[]).integrity(),
            Some(MetaViolation::RebaselineWithoutGates)
        );
        assert_eq!(meta(4, true, &["check", "test"]).integrity(), None);
    }

    #[test]
    fn metadata_round_trips() {
        let m = meta(4, true, &["check", "test"]);
        let text = m.render();
        assert!(text.contains("base_sha=aaaa1111"), "{text}");
        assert_eq!(parse_patch_meta(&text).unwrap(), m);
    }

    #[test]
    fn metadata_without_a_base_is_refused_not_defaulted() {
        let text = meta(4, true, &["check"]).render();
        let without = text.replace("base_sha=aaaa1111 ", "");
        let err = parse_patch_meta(&without).unwrap_err();
        assert!(err.contains("base_sha"), "{err}");
    }

    #[test]
    fn metadata_rejects_garbage() {
        assert!(parse_patch_meta("patch=x").is_err());
        assert!(parse_patch_meta("patch=x nonsense=1").is_err());
        assert!(parse_patch_meta(&format!(
            "patch=x base_sha={BASE} tip_sha={TIP} commits_behind=later rebaselined=1"
        ))
        .is_err());
        assert!(parse_patch_meta(&format!(
            "patch=x base_sha={BASE} tip_sha={TIP} commits_behind=4 rebaselined=maybe"
        ))
        .is_err());
    }

    #[test]
    fn metadata_measures_itself_against_a_later_tip() {
        let m = meta(4, true, &["check"]);
        let d = m.drift_against(TIP).unwrap();
        assert_eq!(d.commits_behind, 4);
        assert!(m.drift_against(BASE).is_err());
    }
}
