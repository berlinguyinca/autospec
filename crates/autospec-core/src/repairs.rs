//! Post-merge repair ratio and gate surface coverage (issue #3858).
//!
//! A *repair* is a merged PR that references a failure, lands within a short
//! window of an earlier merged PR, and touches a file that earlier PR
//! introduced or last modified. The repair ratio per feature — repairs over
//! merged PRs — is the signal that exposes gate/surface mismatch: a feature
//! whose merges keep undoing each other has file types that no gate reads,
//! so nothing between two merges saw the break.
//!
//! [`gate_surface`] answers the review-time question the other way around:
//! for every changed file type, which gate reads it — with the uncovered
//! types named explicitly instead of implied by silence.
//!
//! This is distinct from [`crate::repair_loop`], which counts in-run executor
//! retries. Here the unit of measurement is a merged PR and the evidence is
//! the merge history itself.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

/// Telemetry metric name, emitted alongside the planning counters.
pub const TELEMETRY_METRIC: &str = "autospec.repairs.ratio";

/// Default window: a repair PR lands within 24 h of the PR it repairs.
pub const DEFAULT_REPAIR_WINDOW_SECS: i64 = 24 * 3600;

/// Default threshold: above this share of repair merges a feature raises a
/// finding naming the file types involved.
pub const DEFAULT_REPAIR_THRESHOLD: f64 = 0.20;

/// Case-insensitive stems whose presence in a PR title or body marks the PR
/// as referencing a failure. Intentionally mechanical and recall-oriented:
/// the ratio is an operator-facing signal, not a verdict.
pub const FAILURE_REFERENCES: [&str; 8] = [
    "repair", "fix", "broken", "break", "fail", "regress", "crash", "revert",
];

/// One merged PR as the repair analysis sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPr {
    pub number: u64,
    /// Unix epoch seconds.
    pub merged_at: i64,
    /// Feature/epic scope the PR belongs to.
    pub feature: String,
    pub title: String,
    pub body: String,
    /// Repository-relative paths the PR changed.
    pub files: Vec<String>,
}

/// Classify a changed path into a human file-type label.
///
/// Containerfiles and GitHub workflow files get their own labels before the
/// extension is consulted, because `Dockerfile.prod` and
/// `.github/workflows/ci.yaml` would otherwise classify as `prod`/`yaml`.
pub fn file_type(path: &str) -> String {
    let file = path.rsplit('/').next().unwrap_or(path);
    let lower = file.to_ascii_lowercase();
    if lower.starts_with("dockerfile") || lower.starts_with("containerfile") {
        return "Containerfile".to_string();
    }
    if path.contains(".github/workflows/")
        && matches!(extension_of(&lower), Some("yml") | Some("yaml"))
    {
        return "workflow".to_string();
    }
    match extension_of(&lower) {
        Some("rs") => "Rust".to_string(),
        Some("sh") | Some("bash") => "shell script".to_string(),
        Some("yml") | Some("yaml") => "YAML".to_string(),
        Some("toml") => "TOML".to_string(),
        Some("json") => "JSON".to_string(),
        Some("md") => "Markdown".to_string(),
        Some("py") => ".py".to_string(),
        Some("ts") => ".ts".to_string(),
        Some("js") => ".js".to_string(),
        Some(ext) => format!(".{ext}"),
        None => "unclassified".to_string(),
    }
}

fn extension_of(file: &str) -> Option<&str> {
    let index = file.rfind('.')?;
    let ext = &file[index + 1..];
    (index > 0 && !ext.is_empty()).then_some(ext)
}

/// Static file-type → gate map: which gates read which file type.
///
/// The empty surface asserts that no gate reads anything; the standard
/// surface names the gates the autospec gate set actually runs. The point of
/// the map is to make the *absence* of an entry a named fact instead of a
/// silent gap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateSurface {
    gates: BTreeMap<String, Vec<String>>,
}

impl GateSurface {
    /// A surface with no gates at all.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The standard autospec gate set.
    pub fn standard() -> Self {
        let mut surface = Self::default();
        surface.gates.insert(
            "Rust".to_string(),
            vec![
                "cargo test".to_string(),
                "cargo clippy".to_string(),
                "cargo fmt --check".to_string(),
            ],
        );
        surface.gates.insert(
            "shell script".to_string(),
            vec!["bash -n".to_string(), "shellcheck".to_string()],
        );
        surface
    }

    /// Record the gates that read one file type, replacing any prior entry.
    pub fn with_gates(
        &mut self,
        file_type: impl Into<String>,
        gates: impl IntoIterator<Item = impl Into<String>>,
    ) -> &mut Self {
        self.gates.insert(
            file_type.into(),
            gates.into_iter().map(Into::into).collect(),
        );
        self
    }

    /// The gates that read `file_type`; empty when no gate reads it.
    pub fn gates_for(&self, file_type: &str) -> Vec<String> {
        self.gates.get(file_type).cloned().unwrap_or_default()
    }
}

/// One file type's share of a diff and the gates that read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurfaceEntry {
    pub file_type: String,
    /// Sorted, de-duplicated paths of this type.
    pub files: Vec<String>,
    /// Gates that read this file type; empty when uncovered.
    pub gates: Vec<String>,
}

/// Gate coverage of a set of changed files, one entry per file type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurfaceReport {
    entries: Vec<SurfaceEntry>,
}

impl SurfaceReport {
    /// All entries, in file-type order.
    pub fn entries(&self) -> &[SurfaceEntry] {
        &self.entries
    }

    /// File types no gate reads, in file-type order.
    pub fn uncovered(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| entry.gates.is_empty())
            .map(|entry| entry.file_type.clone())
            .collect()
    }

    /// The review-time line: covered types one per line with their gates,
    /// uncovered types lumped into a single `no gate reads any of these`
    /// clause (largest count first) so the gap is named, not implied.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        let mut uncovered: Vec<&SurfaceEntry> = self
            .entries
            .iter()
            .filter(|entry| entry.gates.is_empty())
            .collect();
        uncovered.sort_by(|a, b| {
            b.files
                .len()
                .cmp(&a.files.len())
                .then_with(|| a.file_type.cmp(&b.file_type))
        });
        for entry in &self.entries {
            if !entry.gates.is_empty() {
                lines.push(format!(
                    "{} — gates: {}",
                    counted(&entry.file_type, entry.files.len()),
                    entry.gates.join(", ")
                ));
            }
        }
        if !uncovered.is_empty() {
            let parts = uncovered
                .iter()
                .map(|entry| counted(&entry.file_type, entry.files.len()))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("{parts} — no gate reads any of these"));
        }
        lines.join("\n")
    }
}

/// `3 Containerfiles`, `1 workflow`, `1 shell script`, `1 .ts`.
fn counted(file_type: &str, count: usize) -> String {
    if count == 1 || file_type.starts_with('.') {
        return format!("{count} {file_type}");
    }
    format!("{count} {file_type}s")
}

/// Group a diff by file type and look each type up in `surface`.
pub fn gate_surface(files: &[String], surface: &GateSurface) -> SurfaceReport {
    let mut by_type: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in files {
        by_type
            .entry(file_type(file))
            .or_default()
            .insert(file.clone());
    }
    let entries = by_type
        .into_iter()
        .map(|(file_type, files)| SurfaceEntry {
            gates: surface.gates_for(&file_type),
            files: files.into_iter().collect(),
            file_type,
        })
        .collect();
    SurfaceReport { entries }
}

/// Whether a PR's title or body references a failure.
pub fn references_failure(pr: &MergedPr) -> bool {
    let haystack = format!("{}\n{}", pr.title.to_lowercase(), pr.body.to_lowercase());
    FAILURE_REFERENCES
        .iter()
        .any(|stem| haystack.contains(stem))
}

fn overlaps(pr: &MergedPr, earlier: &MergedPr) -> bool {
    let earlier_files: BTreeSet<&str> = earlier.files.iter().map(String::as_str).collect();
    pr.files
        .iter()
        .any(|file| earlier_files.contains(file.as_str()))
}

/// The repair PR numbers in a merge-ordered timeline, in merge order.
///
/// A PR is a repair when it references a failure and some strictly earlier
/// PR — within `window_secs` of it — introduced or last modified one of the
/// files it touches.
pub fn classify_repairs(ordered_prs: &[MergedPr], window_secs: i64) -> Vec<u64> {
    let mut repairs = Vec::new();
    for (index, pr) in ordered_prs.iter().enumerate() {
        if !references_failure(pr) {
            continue;
        }
        let is_repair = ordered_prs[..index].iter().any(|earlier| {
            let delta = pr.merged_at - earlier.merged_at;
            delta > 0 && delta <= window_secs && overlaps(pr, earlier)
        });
        if is_repair {
            repairs.push(pr.number);
        }
    }
    repairs
}

/// Repair ratio for one feature.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepairRatio {
    pub feature: String,
    pub merges: u64,
    pub repairs: u64,
    /// Repair PR numbers, in merge order.
    pub repair_prs: Vec<u64>,
    /// Files touched by repair PRs, counted per file type.
    pub files_by_type: BTreeMap<String, u64>,
}

impl RepairRatio {
    /// Repairs over merged PRs for this feature.
    pub fn ratio(&self) -> f64 {
        if self.merges == 0 {
            return 0.0;
        }
        self.repairs as f64 / self.merges as f64
    }

    /// File types the repair PRs touched, in file-type order.
    pub fn file_types(&self) -> Vec<String> {
        self.files_by_type.keys().cloned().collect()
    }
}

/// One repair ratio per feature, in feature order.
///
/// PRs are sorted by `(merged_at, number)` first, so the timeline is
/// deterministic regardless of input order.
pub fn compute_ratios(prs: &[MergedPr], window_secs: i64) -> Vec<RepairRatio> {
    let mut ordered: Vec<MergedPr> = prs.to_vec();
    ordered.sort_by_key(|pr| (pr.merged_at, pr.number));
    let repairs: BTreeSet<u64> = classify_repairs(&ordered, window_secs)
        .into_iter()
        .collect();

    let mut features: BTreeMap<String, Vec<&MergedPr>> = BTreeMap::new();
    for pr in &ordered {
        features.entry(pr.feature.clone()).or_default().push(pr);
    }
    features
        .into_iter()
        .map(|(feature, group)| {
            let mut files_by_type: BTreeMap<String, u64> = BTreeMap::new();
            let mut repair_prs = Vec::new();
            for pr in &group {
                if !repairs.contains(&pr.number) {
                    continue;
                }
                repair_prs.push(pr.number);
                for file in &pr.files {
                    let label = file_type(file);
                    *files_by_type.entry(label).or_default() += 1;
                }
            }
            RepairRatio {
                feature,
                merges: group.len() as u64,
                repairs: repair_prs.len() as u64,
                files_by_type,
                repair_prs,
            }
        })
        .collect()
}

/// One telemetry line per feature, in feature order, ready to sit alongside
/// the planning counters:
/// `autospec.repairs.ratio feature=X merges=8 repairs=3 ratio=0.375`.
pub fn telemetry_lines(ratios: &[RepairRatio]) -> Vec<String> {
    ratios
        .iter()
        .map(|ratio| {
            format!(
                "{} feature={} merges={} repairs={} ratio={:.3}",
                TELEMETRY_METRIC,
                ratio.feature,
                ratio.merges,
                ratio.repairs,
                ratio.ratio()
            )
        })
        .collect()
}

/// A feature whose repair ratio is above threshold.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepairFinding {
    pub feature: String,
    pub ratio: f64,
    pub threshold: f64,
    pub repairs: u64,
    pub merges: u64,
    /// File types the repair PRs touched, in file-type order.
    pub file_types: Vec<String>,
}

/// Findings for every feature strictly above `threshold`, in feature order.
pub fn threshold_findings(ratios: &[RepairRatio], threshold: f64) -> Vec<RepairFinding> {
    ratios
        .iter()
        .filter(|ratio| ratio.ratio() > threshold)
        .map(|ratio| RepairFinding {
            feature: ratio.feature.clone(),
            ratio: ratio.ratio(),
            threshold,
            repairs: ratio.repairs,
            merges: ratio.merges,
            file_types: ratio.file_types(),
        })
        .collect()
}

impl RepairFinding {
    /// The finding as a single operator-facing line.
    pub fn render(&self) -> String {
        let types = if self.file_types.is_empty() {
            "no file types".to_string()
        } else {
            self.file_types.join(", ")
        };
        format!(
            "repair ratio: feature \"{}\" — {} of {} merged PRs ({:.1}%) are repairs, above the {:.1}% threshold; repair PRs touched: {}",
            self.feature,
            self.repairs,
            self.merges,
            self.ratio * 100.0,
            self.threshold * 100.0,
            types
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: u64, merged_at: i64, feature: &str, title: &str, files: &[&str]) -> MergedPr {
        MergedPr {
            number,
            merged_at,
            feature: feature.to_string(),
            title: title.to_string(),
            body: String::new(),
            files: files.iter().map(|file| file.to_string()).collect(),
        }
    }

    fn with_body(mut pr: MergedPr, body: &str) -> MergedPr {
        pr.body = body.to_string();
        pr
    }

    #[test]
    fn file_type_labels_containerfiles_workflows_and_extensions() {
        assert_eq!(file_type("deploy/Dockerfile.prod"), "Containerfile");
        assert_eq!(file_type("app/Containerfile"), "Containerfile");
        assert_eq!(file_type(".github/workflows/ci.yaml"), "workflow");
        assert_eq!(file_type("config/settings.yaml"), "YAML");
        assert_eq!(file_type("scripts/deploy.sh"), "shell script");
        assert_eq!(file_type("frontend/app.ts"), ".ts");
        assert_eq!(file_type("src/lib.rs"), "Rust");
        assert_eq!(file_type("notes/README.md"), "Markdown");
        assert_eq!(file_type("Makefile"), "unclassified");
    }

    #[test]
    fn failure_reference_is_mechanical_and_case_insensitive() {
        assert!(!references_failure(&pr(1, 0, "f", "Re-deployed", &[])));
        assert!(references_failure(&pr(
            1,
            0,
            "f",
            "REPAIR: image is broken",
            &[]
        )));
        assert!(references_failure(&with_body(
            pr(1, 0, "f", "Re-deploy again", &[]),
            "still failing"
        )));
    }

    #[test]
    fn a_repair_needs_overlap_window_and_failure_reference() {
        let earlier = pr(1, 0, "f", "Deploy new image", &["deploy/Dockerfile"]);
        let in_window = pr(
            2,
            3600,
            "f",
            "Re-deploy: image is broken",
            &["deploy/Dockerfile"],
        );
        let out_of_window = pr(
            3,
            99 * 3600,
            "f",
            "Fix broken image",
            &["deploy/Dockerfile"],
        );
        let no_overlap = pr(4, 7200, "f", "Fix broken thing", &["other/file.rs"]);
        let no_reference = pr(5, 7200, "f", "Tweak image", &["deploy/Dockerfile"]);

        assert_eq!(
            classify_repairs(&[earlier.clone(), in_window.clone()], 24 * 3600),
            vec![2]
        );
        assert_eq!(
            classify_repairs(&[earlier.clone(), out_of_window.clone()], 24 * 3600),
            Vec::<u64>::new()
        );
        assert_eq!(
            classify_repairs(&[earlier.clone(), no_overlap.clone()], 24 * 3600),
            Vec::<u64>::new()
        );
        assert_eq!(
            classify_repairs(&[earlier, no_reference], 24 * 3600),
            Vec::<u64>::new()
        );
    }

    #[test]
    fn the_inferweave_chain_is_two_repairs_of_three_merges() {
        let deploy = pr(
            1,
            0,
            "inferweave",
            "Deploy new image",
            &["deploy/Dockerfile.prod"],
        );
        let redeploy = with_body(
            pr(
                2,
                2 * 3600,
                "inferweave",
                "Re-deployed with the old image",
                &[
                    "deploy/Dockerfile.prod",
                    ".github/workflows/deploy.yaml",
                    "scripts/rollback.sh",
                    "frontend/app.ts",
                ],
            ),
            "The new image is broken",
        );
        let still_red = with_body(
            pr(
                3,
                8 * 3600,
                "inferweave",
                "Re-deploy again",
                &["deploy/Dockerfile.prod"],
            ),
            "still failing",
        );

        let ratios = compute_ratios(&[deploy, redeploy, still_red], DEFAULT_REPAIR_WINDOW_SECS);
        assert_eq!(ratios.len(), 1);
        let ratio = &ratios[0];
        assert_eq!(ratio.feature, "inferweave");
        assert_eq!(ratio.merges, 3);
        assert_eq!(ratio.repairs, 2);
        assert_eq!(ratio.repair_prs, vec![2, 3]);
        assert!((ratio.ratio() - 2.0 / 3.0).abs() < f64::EPSILON);
        assert_eq!(
            ratio.file_types(),
            vec![
                ".ts".to_string(),
                "Containerfile".to_string(),
                "shell script".to_string(),
                "workflow".to_string(),
            ]
        );
    }

    #[test]
    fn a_repair_counts_for_its_own_feature_even_across_features() {
        let earlier = pr(10, 0, "calm", "Add deploy", &["scripts/deploy.sh"]);
        let repair = pr(
            11,
            3600,
            "stormy",
            "Fix broken deploy",
            &["scripts/deploy.sh"],
        );
        let ratios = compute_ratios(&[repair, earlier], DEFAULT_REPAIR_WINDOW_SECS);

        assert_eq!(ratios.len(), 2);
        assert_eq!(ratios[0].feature, "calm");
        assert_eq!(ratios[0].repairs, 0);
        assert_eq!(ratios[1].feature, "stormy");
        assert_eq!(ratios[1].merges, 1);
        assert_eq!(ratios[1].repairs, 1);
    }

    #[test]
    fn telemetry_lines_carry_the_metric_name() {
        let ratios = compute_ratios(
            &[pr(1, 0, "a", "Add parser", &["x.rs"])],
            DEFAULT_REPAIR_WINDOW_SECS,
        );
        assert_eq!(
            telemetry_lines(&ratios),
            vec!["autospec.repairs.ratio feature=a merges=1 repairs=0 ratio=0.000".to_string()]
        );
    }

    #[test]
    fn threshold_findings_name_the_file_types() {
        let earlier = pr(
            1,
            0,
            "stormy",
            "Add deploy",
            &["scripts/deploy.sh", "app.ts"],
        );
        let repair = pr(
            2,
            3600,
            "stormy",
            "Fix broken deploy",
            &["scripts/deploy.sh", "app.ts"],
        );
        let ratios = compute_ratios(&[earlier, repair], DEFAULT_REPAIR_WINDOW_SECS);

        let findings = threshold_findings(&ratios, DEFAULT_REPAIR_THRESHOLD);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].feature, "stormy");
        assert_eq!(findings[0].merges, 2);
        assert_eq!(findings[0].repairs, 1);
        assert_eq!(
            findings[0].file_types,
            vec![".ts".to_string(), "shell script".to_string()]
        );
        let line = findings[0].render();
        assert!(line.contains("1 of 2 merged PRs (50.0%)"));
        assert!(line.contains(".ts, shell script"));
    }

    #[test]
    fn at_threshold_is_not_above_threshold() {
        let prs = (0..6)
            .map(|i| pr(i, (i * 100) as i64, "f", "Add thing", &["f.rs"]))
            .chain([
                pr(6, 600, "f", "Fix broken thing", &["f.rs"]),
                pr(7, 700, "f", "Fix another broken thing", &["f.rs"]),
            ])
            .collect::<Vec<_>>();
        let ratios = compute_ratios(&prs, DEFAULT_REPAIR_WINDOW_SECS);
        // Two of eight merges are repairs: exactly 25%, above 20%.
        assert_eq!(threshold_findings(&ratios, 0.25).len(), 0);
        assert_eq!(
            threshold_findings(&ratios, DEFAULT_REPAIR_THRESHOLD).len(),
            1
        );
        // Exactly at 20% (1 of 5 repairs) would not fire: strictly above only.
        let prs = (0..4)
            .map(|i| pr(i, (i * 100) as i64, "g", "Add thing", &["g.rs"]))
            .chain([pr(4, 400, "g", "Fix broken thing", &["g.rs"])])
            .collect::<Vec<_>>();
        let ratios = compute_ratios(&prs, DEFAULT_REPAIR_WINDOW_SECS);
        assert_eq!(
            threshold_findings(&ratios, DEFAULT_REPAIR_THRESHOLD).len(),
            0
        );
    }

    #[test]
    fn surface_report_names_uncovered_types_explicitly() {
        let files = vec![
            "deploy/Dockerfile.base".to_string(),
            "deploy/Dockerfile.api".to_string(),
            "deploy/Dockerfile.web".to_string(),
            ".github/workflows/deploy.yaml".to_string(),
            "scripts/deploy.sh".to_string(),
            "frontend/app.ts".to_string(),
        ];

        let none = gate_surface(&files, &GateSurface::empty());
        assert_eq!(
            none.uncovered(),
            vec![
                ".ts".to_string(),
                "Containerfile".to_string(),
                "shell script".to_string(),
                "workflow".to_string(),
            ]
        );
        assert_eq!(
            none.render(),
            "3 Containerfiles, 1 .ts, 1 shell script, 1 workflow — no gate reads any of these"
        );

        let standard = gate_surface(&files, &GateSurface::standard());
        assert_eq!(
            standard.uncovered(),
            vec![".ts", "Containerfile", "workflow"]
        );
        assert_eq!(
            standard.render(),
            "1 shell script — gates: bash -n, shellcheck\n\
             3 Containerfiles, 1 .ts, 1 workflow — no gate reads any of these"
        );
    }

    #[test]
    fn surface_report_deduplicates_and_sorts_files() {
        let files = vec![
            "a/z.ts".to_string(),
            "a/b.ts".to_string(),
            "a/b.ts".to_string(),
        ];
        let report = gate_surface(&files, &GateSurface::empty());
        assert_eq!(report.entries().len(), 1);
        assert_eq!(
            report.entries()[0].files,
            vec!["a/b.ts".to_string(), "a/z.ts".to_string()]
        );
        assert_eq!(report.render(), "2 .ts — no gate reads any of these");
    }
}
