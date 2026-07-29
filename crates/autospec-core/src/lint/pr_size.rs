use std::collections::BTreeSet;

use super::{normalize_logical_unit, UnifiedDiff};

pub const DEFAULT_MAX_CHANGED_LINES: usize = 400;
pub const DEFAULT_MAX_RAW_FILES: usize = 8;
pub const DEFAULT_MAX_LOGICAL_UNITS: usize = 3;
pub const PROACTIVE_CHANGED_LINES: usize = 320;
pub const PROACTIVE_RAW_FILES: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchSizeLimits {
    pub max_changed_lines: usize,
    pub max_raw_files: usize,
    pub max_logical_units: usize,
}

impl Default for PatchSizeLimits {
    fn default() -> Self {
        Self {
            max_changed_lines: DEFAULT_MAX_CHANGED_LINES,
            max_raw_files: DEFAULT_MAX_RAW_FILES,
            max_logical_units: DEFAULT_MAX_LOGICAL_UNITS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchSize {
    pub changed_lines: usize,
    pub raw_files: usize,
    pub logical_units: usize,
    pub has_binary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchSizeDimension {
    ChangedLines,
    RawFiles,
    LogicalUnits,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSizeEvaluation {
    size: PatchSize,
    hard_dimensions: Vec<PatchSizeDimension>,
    proactive_dimensions: Vec<PatchSizeDimension>,
}

impl PatchSizeEvaluation {
    pub fn size(&self) -> PatchSize {
        self.size
    }

    pub fn hard_dimensions(&self) -> &[PatchSizeDimension] {
        &self.hard_dimensions
    }

    pub fn proactive_dimensions(&self) -> &[PatchSizeDimension] {
        &self.proactive_dimensions
    }

    pub fn is_hard(&self) -> bool {
        !self.hard_dimensions.is_empty()
    }

    pub fn is_proactive(&self) -> bool {
        !self.proactive_dimensions.is_empty()
    }
}

pub fn evaluate_patch_size(diff: &UnifiedDiff, limits: PatchSizeLimits) -> PatchSizeEvaluation {
    let logical_units = diff
        .files
        .iter()
        .filter_map(|file| normalize_logical_unit(&file.path))
        .collect::<BTreeSet<_>>()
        .len();
    let size = PatchSize {
        changed_lines: diff
            .files
            .iter()
            .map(|file| file.changed_line_count())
            .sum(),
        raw_files: diff.files.len(),
        logical_units,
        has_binary: diff.files.iter().any(|file| file.is_binary),
    };
    evaluate_size(size, limits)
}

fn evaluate_size(size: PatchSize, limits: PatchSizeLimits) -> PatchSizeEvaluation {
    let dimensions = [
        (
            PatchSizeDimension::ChangedLines,
            size.changed_lines,
            limits.max_changed_lines,
        ),
        (
            PatchSizeDimension::RawFiles,
            size.raw_files,
            limits.max_raw_files,
        ),
        (
            PatchSizeDimension::LogicalUnits,
            size.logical_units,
            limits.max_logical_units,
        ),
    ];
    let mut hard_dimensions = dimensions
        .iter()
        .filter_map(|(dimension, value, limit)| (*value > *limit).then_some(*dimension))
        .collect::<Vec<_>>();
    if size.has_binary {
        hard_dimensions.push(PatchSizeDimension::Binary);
    }
    let proactive_dimensions = [
        (
            PatchSizeDimension::ChangedLines,
            size.changed_lines >= PROACTIVE_CHANGED_LINES,
        ),
        (
            PatchSizeDimension::RawFiles,
            size.raw_files >= PROACTIVE_RAW_FILES,
        ),
    ]
    .into_iter()
    .filter_map(|(dimension, reached)| reached.then_some(dimension))
    .collect();

    PatchSizeEvaluation {
        size,
        hard_dimensions,
        proactive_dimensions,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_patch_size, evaluate_size, PatchSize, PatchSizeDimension, PatchSizeLimits,
    };
    use crate::lint::{parse_unified_diff, DiffFile, UnifiedDiff};

    fn evaluate(
        changed_lines: usize,
        raw_files: usize,
        logical_units: usize,
    ) -> super::PatchSizeEvaluation {
        evaluate_size(
            PatchSize {
                changed_lines,
                raw_files,
                logical_units,
                has_binary: false,
            },
            PatchSizeLimits::default(),
        )
    }

    #[test]
    fn hard_limits_are_inclusive() {
        assert!(evaluate(400, 8, 3).hard_dimensions().is_empty());
        assert_eq!(
            evaluate(401, 8, 3).hard_dimensions(),
            &[PatchSizeDimension::ChangedLines]
        );
        assert_eq!(
            evaluate(400, 9, 3).hard_dimensions(),
            &[PatchSizeDimension::RawFiles]
        );
        assert_eq!(
            evaluate(400, 8, 4).hard_dimensions(),
            &[PatchSizeDimension::LogicalUnits]
        );
    }

    #[test]
    fn proactive_limits_ignore_logical_units() {
        assert!(evaluate(320, 1, 1).is_proactive());
        assert!(evaluate(1, 7, 1).is_proactive());
        assert!(!evaluate(1, 1, 3).is_proactive());
        assert_eq!(
            evaluate(1, 1, 4).hard_dimensions(),
            &[PatchSizeDimension::LogicalUnits]
        );
        assert!(!evaluate(319, 6, 2).is_proactive());
    }

    #[test]
    fn additions_and_removals_both_count_as_changed_lines() {
        let diff = parse_unified_diff(concat!(
            "diff ",
            "--git a/src/lib.rs b/src/lib.rs\n\
             --- a/src/lib.rs\n\
             +++ b/src/lib.rs\n\
             @@ -1 +1 @@\n\
             -old\n\
             +new\n"
        ))
        .expect("diff parses");

        assert_eq!(
            evaluate_patch_size(&diff, PatchSizeLimits::default())
                .size()
                .changed_lines,
            2
        );
    }

    #[test]
    fn adapter_trio_and_derived_golden_are_one_logical_unit() {
        let files = [
            "skills/autospec/SKILL.md",
            "skills/autospec/codex/prompt.md",
            "skills/autospec/opencode/agent.md",
            "tests/fixtures/skill-goldens/autospec.sha256",
        ]
        .into_iter()
        .map(empty_file)
        .collect();

        assert_eq!(
            evaluate_patch_size(&UnifiedDiff { files }, PatchSizeLimits::default())
                .size()
                .logical_units,
            1
        );
    }

    #[test]
    fn binary_markers_are_always_hard() {
        for marker in [
            "Binary files a/image.png and b/image.png differ",
            "GIT binary patch",
        ] {
            let source = format!(
                concat!("diff ", "--git a/image.png b/image.png\n{}\n"),
                marker
            );
            let diff = parse_unified_diff(&source).expect("binary diff parses");
            assert_eq!(
                evaluate_patch_size(&diff, PatchSizeLimits::default()).hard_dimensions(),
                &[PatchSizeDimension::Binary]
            );
        }
    }

    fn empty_file(path: &str) -> DiffFile {
        DiffFile {
            path: path.to_string(),
            is_new: false,
            is_binary: false,
            hunks: Vec::new(),
        }
    }
}
