//! Per-pipeline coverage for the conversion pass (issue #4556).
//!
//! The pass is specified over `$L/*/out/issue-*/changes.patch` — a glob across
//! every pipeline — but a run is handed one pipeline's root
//! (`--llm-root $L/<pipeline>`). Measured on the fleet: four pipelines, one
//! reached, 110 patches in the other three never examined — and the run
//! reported a clean `examined=…` line that read as if the backlog were empty.
//!
//! The defect this closes has the shape of #4449: a wrong answer that looked
//! like a clean one. A count of zero from a directory that was never opened
//! must not be reportable as "nothing to convert", and a run that reaches 1 of
//! 4 pipelines must not report success. The pass now says, on every run, which
//! pipelines it reached and how many patches the unreached ones hold.

use std::path::Path;

/// The exit status a pass with incomplete coverage exits with: the work it did
/// was done, the counters are true — but the pass's nominal scope (every
/// pipeline under its glob) was not reached, so the run is not a success.
/// Distinct from 2 (a diagnostic: the pass could not run) because the pass
/// ran; distinct from 0 because it was not whole.
pub const INCOMPLETE_EXIT_CODE: i32 = 3;

/// One pipeline under the pass's glob root: its name and how many patches it
/// holds on disk right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineCount {
    pub name: String,
    pub patches: usize,
}

/// The pass's coverage of its glob: which pipelines exist, which one this run
/// reached, and how many patches each holds. `pipelines` is sorted by name and
/// includes the reached pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageReport {
    /// The name of the pipeline this run reached, or `all` when the run was
    /// handed the shared root and enumerated every pipeline itself.
    pub reached: String,
    pub pipelines: Vec<PipelineCount>,
}

/// `issue-<N>` → `N`. Anything that is not exactly that shape is not a patch
/// directory: the same rule the pass's own enumeration applies, so the
/// coverage count and the examined count cannot disagree about what a patch is.
fn parse_issue_name(name: &str) -> Option<u64> {
    let number = name.strip_prefix("issue-")?;
    let number = number.parse::<u64>().ok()?;
    (number > 0).then_some(number)
}

/// How many `out/issue-*/changes.patch` files a pipeline directory holds.
/// A missing or unreadable `out` directory counts as zero (the pipeline
/// simply has no patches on disk right now).
/// The patches a node directory holds: `out/issue-*/changes.patch`. A
/// missing or unreadable `out` directory counts as zero (the node simply has
/// no patches on disk right now).
fn node_patches(node: &Path) -> usize {
    let out_dir = node.join("out");
    let Ok(issues) = std::fs::read_dir(&out_dir) else {
        return 0;
    };
    issues
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(parse_issue_name)
                .is_some()
        })
        .filter(|entry| entry.path().join("changes.patch").is_file())
        .count()
}

/// How many patches a pipeline directory holds. A pipeline may hold them
/// directly (`pipeline/out/issue-*` — the shared-root shape, where the
/// pipeline is the node) or through node directories
/// (`pipeline/<node>/out/issue-*` — the shape the pass's own enumeration
/// reads when it is handed one pipeline's root). Both shapes occur on the
/// fleet, and a pipeline with neither holds zero.
pub fn count_patches(pipeline: &Path) -> usize {
    let mut count = node_patches(pipeline);
    if let Ok(entries) = std::fs::read_dir(pipeline) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                count += node_patches(&entry.path());
            }
        }
    }
    count
}

/// Whether a directory could hold a pipeline's patches: it (or one of its
/// direct children) has an `out` directory. The `out` directory is what the
/// pass's glob names — a sibling that has neither was never a pipeline to
/// begin with, and an empty one does not make the run incomplete.
fn is_pipeline(entry: &std::fs::DirEntry) -> bool {
    let Ok(path) = entry.path().canonicalize() else {
        return false;
    };
    if !path.is_dir() {
        return false;
    }
    if path.join("out").is_dir() {
        return true;
    }
    std::fs::read_dir(&path)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.path().is_dir() && e.path().join("out").is_dir())
        })
        .unwrap_or(false)
}

/// Build the coverage report for the root the pass was handed.
///
/// The tree shape is the operator's to declare (`shared`, from
/// `--shared-llm-root`) because the two shapes are structurally identical at
/// the root — a pipeline's node directories look exactly like a shared
/// root's pipeline directories:
///
/// - `shared = false` (the run's usual shape): the root is one pipeline's
///   directory; the other pipelines are its siblings under the same parent.
///   Coverage is complete only if every sibling holds zero patches. Fewer
///   than two sibling pipelines: no coverage question (`None`).
/// - `shared = true` (the run was handed the shared parent, e.g. `$L`):
///   the root's children that hold `out` directly are the pipelines, and the
///   pass's own enumeration read every one of them — complete by
///   construction (`reached = "all"`).
pub fn build(root: &Path, shared: bool) -> Option<CoverageReport> {
    let reached_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let counts_of = |entries: &[&std::fs::DirEntry]| -> Vec<PipelineCount> {
        let mut counts: Vec<PipelineCount> = entries
            .iter()
            .map(|e| PipelineCount {
                name: e.file_name().to_string_lossy().into_owned(),
                patches: count_patches(&e.path()),
            })
            .collect();
        counts.sort_by(|a, b| a.name.cmp(&b.name));
        counts
    };
    let root_children: Vec<std::fs::DirEntry> = std::fs::read_dir(root).ok()?.flatten().collect();
    if shared {
        // The shared root's pipelines hold `out` directly
        // (root/<pipeline>/out/issue-*); the pass enumerated every one.
        let own_pipelines: Vec<&std::fs::DirEntry> = root_children
            .iter()
            .filter(|e| e.path().is_dir() && e.path().join("out").is_dir())
            .collect();
        return Some(CoverageReport {
            reached: "all".to_string(),
            pipelines: counts_of(&own_pipelines),
        });
    }
    // The root is one pipeline: its siblings under the same parent are the
    // rest of the glob.
    let parent = root.parent()?;
    let siblings: Vec<std::fs::DirEntry> = std::fs::read_dir(parent).ok()?.flatten().collect();
    let pipeline_entries: Vec<&std::fs::DirEntry> =
        siblings.iter().filter(|e| is_pipeline(e)).collect();
    (pipeline_entries.len() >= 2).then_some(CoverageReport {
        reached: reached_name,
        pipelines: counts_of(&pipeline_entries),
    })
}

impl CoverageReport {
    /// The pipelines this run did not reach, in sorted order. A run handed
    /// the shared root (`reached == "all"`) reached every pipeline by
    /// construction, so it has none left out.
    pub fn unreached(&self) -> Vec<&PipelineCount> {
        if self.reached == "all" {
            return Vec::new();
        }
        self.pipelines
            .iter()
            .filter(|p| p.name != self.reached)
            .collect()
    }

    /// The patches held by the pipelines this run never opened.
    pub fn unreached_patches(&self) -> usize {
        self.unreached().iter().map(|p| p.patches).sum()
    }

    /// Whether every pipeline under the glob was reached: no unreached
    /// pipeline holds a patch. An unreached pipeline with an empty `out`
    /// directory holds nothing, so it does not make the run incomplete —
    /// there was nothing to examine there.
    pub fn complete(&self) -> bool {
        self.unreached_patches() == 0
    }

    /// The suffix appended to the pass's summary line: `coverage=R/T
    /// pipelines`. `T` is the number of pipelines under the glob, `R` how
    /// many of them hold no patch this run skipped over — on the reached
    /// root that is the reached pipeline plus every empty sibling.
    pub fn suffix(&self) -> String {
        let reached_count = if self.reached == "all" {
            self.pipelines.len()
        } else {
            self.pipelines
                .iter()
                .filter(|p| p.name == self.reached || p.patches == 0)
                .count()
        };
        format!(
            "coverage={}/{} pipelines",
            reached_count,
            self.pipelines.len()
        )
    }

    /// One warning per unreached pipeline that holds patches — the lines that
    /// keep "nothing to convert" unreportable when 110 patches were never
    /// opened.
    pub fn warnings(&self) -> Vec<String> {
        self.unreached()
            .iter()
            .filter(|p| p.patches > 0)
            .map(|p| {
                format!(
                    "coverage gap: pipeline '{}' holds {} patch(es) this run never \
                     examined (reached '{}') — point the pass at it or its shared \
                     root, or archive what will never convert",
                    p.name, p.patches, self.reached
                )
            })
            .collect()
    }

    /// The exit status the run earns, or `None` when coverage is complete.
    pub fn exit_status(&self) -> Option<i32> {
        (!self.complete()).then_some(INCOMPLETE_EXIT_CODE)
    }

    /// The status line printed with the incomplete exit: what the run did
    /// reach, and what it left unexamined.
    pub fn status_line(&self) -> String {
        let unreached: Vec<&PipelineCount> = self
            .unreached()
            .iter()
            .filter(|p| p.patches > 0)
            .copied()
            .collect();
        format!(
            "conversion pass incomplete: reached '{}' — {} of {} pipeline(s) hold \
             {} unexamined patch(es) ({})",
            self.reached,
            unreached.len(),
            self.pipelines.len(),
            self.unreached_patches(),
            unreached
                .iter()
                .map(|p| format!("{}:{}", p.name, p.patches))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "autospec-pipeline-coverage-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn make_patch(root: &Path, pipeline: &str, issue: u64) {
        let issue_dir = root
            .join(pipeline)
            .join("out")
            .join(format!("issue-{issue}"));
        fs::create_dir_all(&issue_dir).expect("issue dir");
        fs::write(issue_dir.join("changes.patch"), "patch\n").expect("patch");
    }

    #[test]
    fn a_root_with_one_pipeline_has_no_coverage_question() {
        // The parent holds exactly one pipeline: nothing was left out.
        let parent = temp_root("single");
        make_patch(&parent, "autospec", 1);
        assert!(build(parent.join("autospec").as_path(), false).is_none());
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_shared_root_reaches_every_pipeline_by_construction() {
        let root = temp_root("shared");
        make_patch(&root, "autospec", 1);
        make_patch(&root, "iw", 2);
        let report = build(&root, true).expect("shared root reports coverage");
        assert_eq!(report.reached, "all");
        assert_eq!(report.pipelines.len(), 2);
        assert!(report.complete());
        assert_eq!(report.suffix(), "coverage=2/2 pipelines");
        assert!(report.warnings().is_empty());
        assert_eq!(report.exit_status(), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn one_of_four_reached_is_incomplete_and_says_which_pipelines_it_left() {
        // A dedicated parent: the sibling scan must see exactly these four.
        let root = temp_root("four");
        make_patch(&root, "autospec", 1);
        for issue in 2..12 {
            make_patch(&root, "iw", issue);
        }
        make_patch(&root, "disp", 20);
        make_patch(&root, "orch", 30);
        let report = build(root.join("autospec").as_path(), false).expect("coverage");
        assert_eq!(report.reached, "autospec");
        assert_eq!(report.pipelines.len(), 4);
        assert!(!report.complete());
        assert_eq!(report.suffix(), "coverage=1/4 pipelines");
        assert_eq!(report.unreached_patches(), 12);
        let warnings = report.warnings();
        assert_eq!(warnings.len(), 3);
        assert!(warnings[0].contains("pipeline 'disp'"), "{warnings:?}");
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("pipeline 'iw' holds 10")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("pipeline 'orch'")),
            "{warnings:?}"
        );
        assert_eq!(report.exit_status(), Some(INCOMPLETE_EXIT_CODE));
        let status = report.status_line();
        assert!(
            status.starts_with("conversion pass incomplete: reached 'autospec'"),
            "{status}"
        );
        assert!(status.contains("iw:10"), "{status}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unreached_pipeline_with_no_patches_does_not_block_completeness() {
        let root = temp_root("empty-sibling"); // dedicated parent, as above
        make_patch(&root, "autospec", 1);
        fs::create_dir_all(root.join("orch").join("out")).expect("empty pipeline");
        let report = build(root.join("autospec").as_path(), false).expect("coverage");
        assert!(
            report.complete(),
            "an empty sibling holds nothing to examine"
        );
        assert_eq!(report.suffix(), "coverage=2/2 pipelines");
        assert!(report.warnings().is_empty());
        assert_eq!(report.exit_status(), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn non_patch_entries_are_not_counted() {
        let root = temp_root("noise");
        let iw = root.join("iw").join("out");
        fs::create_dir_all(iw.join("issue-0")).expect("issue-0");
        fs::create_dir_all(iw.join("issue-x")).expect("issue-x");
        fs::create_dir_all(iw.join("scratch")).expect("scratch");
        fs::create_dir_all(iw.join("issue-7")).expect("issue-7");
        fs::write(iw.join("issue-7").join("other.patch"), "not the name\n").expect("other");
        assert_eq!(count_patches(root.join("iw").as_path()), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_root_with_no_parent_reports_nothing() {
        // A filesystem root has no parent to look at: no coverage question.
        assert!(build(Path::new("/"), false).is_none());
    }
}
