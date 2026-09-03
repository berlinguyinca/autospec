//! Persistent worktree task memory (AAR spec section 7).
//!
//! Durable facts live on disk in the worktree, not in an ever-growing
//! conversation. This module is pure: it renders and parses the files and
//! reports what should exist; the driver performs the writes.

use std::collections::BTreeMap;

/// Directory, relative to the worktree root, holding AAR task memory.
pub const MEMORY_DIR: &str = ".autospec";

/// Telemetry subdirectory inside the memory directory.
pub const TELEMETRY_DIR: &str = ".autospec/telemetry";

/// Default line budget per memory file.
///
/// Memory that grows without bound recreates the problem it exists to solve:
/// a file that no longer fits in context is not durable memory.
pub const DEFAULT_MAX_LINES: usize = 120;

/// The durable memory files an AAR worktree carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryFile {
    Task,
    Plan,
    State,
    Findings,
    Decisions,
    Tests,
    Review,
}

impl MemoryFile {
    pub fn all() -> [MemoryFile; 7] {
        [
            MemoryFile::Task,
            MemoryFile::Plan,
            MemoryFile::State,
            MemoryFile::Findings,
            MemoryFile::Decisions,
            MemoryFile::Tests,
            MemoryFile::Review,
        ]
    }

    pub fn file_name(&self) -> &'static str {
        match self {
            MemoryFile::Task => "task.md",
            MemoryFile::Plan => "plan.md",
            MemoryFile::State => "state.md",
            MemoryFile::Findings => "findings.md",
            MemoryFile::Decisions => "decisions.md",
            MemoryFile::Tests => "tests.md",
            MemoryFile::Review => "review.md",
        }
    }

    pub fn relative_path(&self) -> String {
        format!("{MEMORY_DIR}/{}", self.file_name())
    }

    pub fn heading(&self) -> &'static str {
        match self {
            MemoryFile::Task => "Task",
            MemoryFile::Plan => "Plan",
            MemoryFile::State => "State",
            MemoryFile::Findings => "Findings",
            MemoryFile::Decisions => "Decisions",
            MemoryFile::Tests => "Tests",
            MemoryFile::Review => "Review",
        }
    }

    /// One line telling an agent what belongs in the file.
    pub fn purpose(&self) -> &'static str {
        match self {
            MemoryFile::Task => "The work item, its acceptance criteria, and its scope boundary.",
            MemoryFile::Plan => "The ordered steps agreed for this task; one logical change each.",
            MemoryFile::State => "Where execution actually is right now, and what is next.",
            MemoryFile::Findings => "Established facts about the code, with the evidence for each.",
            MemoryFile::Decisions => "Choices made, the alternative rejected, and why.",
            MemoryFile::Tests => "Commands run, their outcome, and what they proved.",
            MemoryFile::Review => "Review outcomes and the rework each one required.",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().trim_end_matches(".md") {
            "task" => MemoryFile::Task,
            "plan" => MemoryFile::Plan,
            "state" => MemoryFile::State,
            "findings" => MemoryFile::Findings,
            "decisions" => MemoryFile::Decisions,
            "tests" => MemoryFile::Tests,
            "review" => MemoryFile::Review,
            _ => return None,
        })
    }
}

/// One durable entry: a bullet with an optional evidence pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEntry {
    pub text: String,
    pub evidence: Option<String>,
}

impl MemoryEntry {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            evidence: None,
        }
    }

    pub fn with_evidence(text: impl Into<String>, evidence: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            evidence: Some(evidence.into()),
        }
    }

    fn render(&self) -> String {
        match &self.evidence {
            Some(evidence) => format!("- {} ({evidence})", self.text),
            None => format!("- {}", self.text),
        }
    }

    fn parse(line: &str) -> Option<Self> {
        let body = line.trim().strip_prefix("- ")?.trim();
        if body.is_empty() {
            return None;
        }
        if let Some(open) = body.rfind(" (") {
            if body.ends_with(')') {
                let text = body[..open].trim().to_string();
                let evidence = body[open + 2..body.len() - 1].trim().to_string();
                if !text.is_empty() && !evidence.is_empty() {
                    return Some(Self {
                        text,
                        evidence: Some(evidence),
                    });
                }
            }
        }
        Some(Self {
            text: body.to_string(),
            evidence: None,
        })
    }
}

/// In-memory view of a worktree's durable task memory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorktreeMemory {
    entries: BTreeMap<MemoryFile, Vec<MemoryEntry>>,
    pub max_lines: usize,
}

impl WorktreeMemory {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            max_lines: DEFAULT_MAX_LINES,
        }
    }

    pub fn append(&mut self, file: MemoryFile, entry: MemoryEntry) {
        self.entries.entry(file).or_default().push(entry);
    }

    pub fn entries(&self, file: MemoryFile) -> &[MemoryEntry] {
        self.entries.get(&file).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Replace a file's entries, e.g. when state advances.
    pub fn replace(&mut self, file: MemoryFile, entries: Vec<MemoryEntry>) {
        self.entries.insert(file, entries);
    }

    pub fn render(&self, file: MemoryFile) -> String {
        let mut output = format!("# {}\n\n<!-- {} -->\n", file.heading(), file.purpose());
        let entries = self.entries(file);
        if entries.is_empty() {
            output.push_str("\n_No entries recorded._\n");
            return output;
        }
        output.push('\n');
        for entry in entries {
            output.push_str(&entry.render());
            output.push('\n');
        }
        output
    }

    /// Every memory file with its rendered contents, relative to the worktree.
    pub fn rendered_files(&self) -> Vec<(String, String)> {
        MemoryFile::all()
            .into_iter()
            .map(|file| (file.relative_path(), self.render(file)))
            .collect()
    }

    /// Files that exceed the line budget and need summarizing.
    pub fn over_budget(&self) -> Vec<MemoryFile> {
        MemoryFile::all()
            .into_iter()
            .filter(|file| self.render(*file).lines().count() > self.max_lines)
            .collect()
    }

    /// Parse one rendered memory file back into entries.
    pub fn load(&mut self, file: MemoryFile, contents: &str) {
        let entries = contents
            .lines()
            .filter(|line| !line.trim_start().starts_with("<!--"))
            .filter_map(MemoryEntry::parse)
            .collect::<Vec<_>>();
        self.entries.insert(file, entries);
    }
}

/// Directories an AAR worktree needs, relative to its root.
pub fn required_directories() -> Vec<&'static str> {
    vec![MEMORY_DIR, TELEMETRY_DIR]
}

/// Scaffold contents for a fresh worktree.
pub fn scaffold() -> Vec<(String, String)> {
    WorktreeMemory::new().rendered_files()
}
