//! A small, dependency-free unified-diff model for deterministic policy checks.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedDiff {
    pub files: Vec<DiffFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub is_new: bool,
    pub hunks: Vec<DiffHunk>,
}

impl DiffFile {
    pub fn added_lines(&self) -> impl Iterator<Item = &DiffLine> {
        self.hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .filter(|line| line.kind == DiffLineKind::Added)
    }

    pub fn added_line_count(&self) -> usize {
        self.added_lines().count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub old_start: usize,
    pub new_start: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

/// Parse the subset of unified diff syntax consumed by the lint policy.
///
/// It intentionally accepts ordinary git diffs without depending on `git` or
/// the filesystem. Malformed hunk headers are reported rather than guessed.
pub fn parse_unified_diff(source: &str) -> Result<UnifiedDiff, String> {
    let mut files = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    let finish_hunk = |file: &mut DiffFile, hunk: &mut Option<DiffHunk>| {
        if let Some(hunk) = hunk.take() {
            file.hunks.push(hunk);
        }
    };
    let finish_file =
        |files: &mut Vec<DiffFile>, file: &mut Option<DiffFile>, hunk: &mut Option<DiffHunk>| {
            if let Some(mut file) = file.take() {
                finish_hunk(&mut file, hunk);
                files.push(file);
            }
        };

    for raw in source.lines() {
        if let Some(path) = raw.strip_prefix("diff --git a/") {
            finish_file(&mut files, &mut current_file, &mut current_hunk);
            let (_, path) = path
                .split_once(" b/")
                .ok_or_else(|| format!("malformed diff header: {raw}"))?;
            current_file = Some(DiffFile {
                path: path.to_string(),
                is_new: false,
                hunks: Vec::new(),
            });
            continue;
        }

        let Some(file) = current_file.as_mut() else {
            continue;
        };

        if raw == "new file mode 100644" || raw.starts_with("new file mode ") {
            file.is_new = true;
            continue;
        }
        if raw.starts_with("@@ ") {
            finish_hunk(file, &mut current_hunk);
            let (old_start, new_start) = parse_hunk_header(raw)?;
            current_hunk = Some(DiffHunk {
                old_start,
                new_start,
                lines: Vec::new(),
            });
            continue;
        }
        if raw.starts_with("--- ") || raw.starts_with("+++ ") || raw.starts_with("\\ No newline") {
            continue;
        }

        let Some(hunk) = current_hunk.as_mut() else {
            continue;
        };
        let (kind, content) = match raw.as_bytes().first() {
            Some(b' ') => (DiffLineKind::Context, &raw[1..]),
            Some(b'+') => (DiffLineKind::Added, &raw[1..]),
            Some(b'-') => (DiffLineKind::Removed, &raw[1..]),
            _ => continue,
        };
        let old_line = match kind {
            DiffLineKind::Added => None,
            DiffLineKind::Context | DiffLineKind::Removed => Some(
                hunk.old_start
                    + hunk
                        .lines
                        .iter()
                        .filter(|line| line.kind != DiffLineKind::Added)
                        .count(),
            ),
        };
        let new_line = match kind {
            DiffLineKind::Removed => None,
            DiffLineKind::Context | DiffLineKind::Added => Some(
                hunk.new_start
                    + hunk
                        .lines
                        .iter()
                        .filter(|line| line.kind != DiffLineKind::Removed)
                        .count(),
            ),
        };
        hunk.lines.push(DiffLine {
            kind,
            content: content.to_string(),
            old_line,
            new_line,
        });
    }
    finish_file(&mut files, &mut current_file, &mut current_hunk);
    Ok(UnifiedDiff { files })
}

fn parse_hunk_header(header: &str) -> Result<(usize, usize), String> {
    let mut fields = header.split_whitespace();
    if fields.next() != Some("@@") {
        return Err(format!("malformed hunk header: {header}"));
    }
    let old = fields
        .next()
        .ok_or_else(|| format!("malformed hunk header: {header}"))?;
    let new = fields
        .next()
        .ok_or_else(|| format!("malformed hunk header: {header}"))?;
    if fields.next() != Some("@@") {
        return Err(format!("malformed hunk header: {header}"));
    }
    Ok((parse_range(old, '-')?, parse_range(new, '+')?))
}

fn parse_range(range: &str, prefix: char) -> Result<usize, String> {
    let value = range
        .strip_prefix(prefix)
        .ok_or_else(|| format!("malformed hunk range: {range}"))?;
    value
        .split_once(',')
        .map_or(value, |(start, _)| start)
        .parse::<usize>()
        .map_err(|_| format!("malformed hunk range: {range}"))
}
