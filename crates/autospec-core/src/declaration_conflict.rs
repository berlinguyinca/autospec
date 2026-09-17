//! Conflicts confined to declaration lines, and their canonical union
//! (issue #4463).
//!
//! A conversion batch of 13 Rust patches against clean `origin/main` produced
//! three applies and ten conflicts. **Six of the ten conflicted in the same
//! place**: `crates/autospec-core/src/lib.rs`, `insights/mod.rs`,
//! `coordination/mod.rs` — none of the patches overlapping in the code they
//! added. Each added its own module in its own file; they collided because
//! Rust requires the module to be declared, every declaration goes into the
//! same list, and every agent appends to the same region of the same file.
//!
//! The module list is a shared mutable index that nothing owns. It is not
//! where the work is, but it is where all the work has to arrive. The
//! contention scales with the number of parallel agents, and it worsens with
//! age: the longer a patch sits unconverted, the more declarations have landed
//! ahead of it, so the oldest patches are the most likely to conflict and the
//! backlog becomes self-reinforcing.
//!
//! The invariant this module serves: **a file that every change must touch but
//! no change is about is a coordination point, and its conflicts must be
//! resolvable without judgement.**
//!
//! What is decided here, and what is not:
//!
//! 1. **Whether a conflict is confined to declarations** ([`detect`]). A hunk
//!    whose every line is a `mod` declaration (with its `#[cfg]` attributes)
//!    needs no judgement: both sides want an entry in an index. A hunk
//!    containing one line of real code is [`Conflict::Mixed`] and is not this
//!    module's business — merging that would be exactly the judgement the
//!    invariant forbids automating.
//! 2. **The union, in canonical order** ([`sorted_union`]). Order is the second
//!    half of the defect. Taking "both sides, first-seen" leaves the index in
//!    the order whoever merged last appended it, so the next patch still lands
//!    at the end of the list and still collides. A canonical position means two
//!    independent additions land at different offsets and git resolves them
//!    with no help: the contention stops instead of being resolved ever more
//!    cleverly.
//! 3. **A union that names a module nothing provides is refused**
//!    ([`orphaned`]). Classic two-sided markers cannot distinguish "this side
//!    added `foo`" from "that side deleted `foo`", and a union resurrects the
//!    deleted declaration. That is not a reason to abandon the union, it is a
//!    reason to check it: `pub mod foo;` needs `foo.rs` or `foo/mod.rs` in the
//!    merged tree, and absence is decidable. Refusing there is the difference
//!    between an auto-resolution and an auto-build-failure.
//!
//! Everything is pure, including the filesystem question, which is answered by
//! a caller-supplied predicate so the rule is testable without a checkout.

/// One line of a conflicted file, as it bears on a declaration union.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line<'a> {
    /// An attribute (`#[cfg(test)]`, `#[cfg(feature = "x")]`) belonging to the
    /// declaration that follows it.
    Attribute(&'a str),
    /// A module declaration: `mod x;`, `pub mod x;`, `pub(crate) mod x;`.
    Declaration(&'a str),
    /// Anything else: real code, a comment, a `use`, a blank line, or an
    /// inner attribute that belongs to the enclosing module rather than to the
    /// next declaration.
    Other(&'a str),
}

/// The declaration prefixes this module is willing to union.
const DECLARATION_PREFIXES: [&str; 4] = ["pub(crate) mod ", "pub(super) mod ", "pub mod ", "mod "];

/// Classify one line of a conflicted file.
///
/// An *inline* module (`mod support {`) is deliberately [`Line::Other`]: its
/// closing brace is not a declaration, so a hunk containing one cannot be
/// unioned line-wise without cutting a block in half.
pub fn classify_line(line: &str) -> Line<'_> {
    let text = line.trim();
    if text.starts_with("#![") {
        return Line::Other(line);
    }
    if text.starts_with("#[") {
        return Line::Attribute(line);
    }
    match declared_module(text) {
        Some(_) => Line::Declaration(line),
        None => Line::Other(line),
    }
}

/// The module a single declaration line names, or `None` when the line is not
/// a plain module declaration.
fn declared_module(text: &str) -> Option<&str> {
    if text.contains('{') || !text.ends_with(';') {
        return None;
    }
    for prefix in DECLARATION_PREFIXES {
        if let Some(rest) = text.strip_prefix(prefix) {
            let name = rest.trim().trim_end_matches(';').trim();
            if name.is_empty() || name.contains(' ') || name.contains("::") {
                return None;
            }
            return Some(name);
        }
    }
    None
}

/// Every module declared in `text`, for the "does a file provide it" check.
pub fn declared_modules(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| declared_module(line.trim()))
        .map(str::to_string)
        .collect()
}

/// What the conflict hunks of a file contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// The file carries no conflict markers: git left it unmerged for a reason
    /// this module cannot see, so nothing here applies.
    NoConflict,
    /// Every line inside every hunk is a module declaration or an attribute
    /// attached to one. The resolution needs no judgement.
    DeclarationOnly,
    /// At least one hunk contains something other than a declaration. That is a
    /// conflict about code, and it is resolved by a reader, not by a union.
    Mixed,
}

impl Conflict {
    /// Whether the pass may take the union without holding.
    pub fn is_declaration_only(self) -> bool {
        self == Self::DeclarationOnly
    }

    /// The phrase recorded on a held patch, so the size of this class is
    /// measured rather than sampled from an incident (#4463's third
    /// acceptance criterion).
    pub fn note(self) -> &'static str {
        match self {
            Self::DeclarationOnly => ", declaration-only",
            Self::NoConflict | Self::Mixed => "",
        }
    }
}

/// Why a declaration union could not be computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnionError {
    /// The file has no conflict markers.
    NoConflict,
    /// A hunk contains something that is not a declaration: unioning it would
    /// be an unjudged merge of code.
    NotDeclarationOnly,
    /// A hunk's markers do not form a well-formed region.
    UnbalancedMarkers {
        /// what the walk found.
        detail: String,
    },
}

/// Git's classic conflict markers, matching [`crate::conflict_merge`].
const OPEN: &str = "<<<<<<< ";
const SEP: &str = "=======";
const CLOSE: &str = ">>>>>>> ";
const DIFF3: &str = "|||||||";

/// Inspect the hunks of a conflicted file without changing it.
pub fn detect(content: &str) -> Conflict {
    match hunks(content) {
        Ok(None) => Conflict::NoConflict,
        // Malformed markers are never treated as safe.
        Err(_) => Conflict::Mixed,
        Ok(Some(regions)) => match declaration_only_regions(&regions) {
            true => Conflict::DeclarationOnly,
            false => Conflict::Mixed,
        },
    }
}

/// Whether every hunk is confined to portable declarations. A hunk with no
/// content on either side has nothing to union, so it is not a declaration
/// conflict either.
fn declaration_only_regions(regions: &[Vec<String>]) -> bool {
    regions.iter().all(|hunk| {
        hunk.iter().any(|line| !line.trim().is_empty()) && hunk.iter().all(|line| is_portable(line))
    })
}

/// Whether a line may be moved between sides of a hunk.
fn is_portable(line: &str) -> bool {
    line.trim().is_empty()
        || matches!(
            classify_line(line),
            Line::Declaration(_) | Line::Attribute(_)
        )
}

/// Replace every conflict hunk with the canonical union of the declarations
/// both sides asked for: every declaration present on either side, sorted,
/// each attribute kept attached to the declaration it precedes.
///
/// Lines outside the hunks pass through byte-for-byte. The order is a property
/// of the module names rather than of merge order, which is what makes the next
/// independent addition land somewhere else instead of at the same offset.
pub fn sorted_union(content: &str) -> Result<String, UnionError> {
    let regions = match hunks(content)? {
        None => return Err(UnionError::NoConflict),
        Some(regions) => regions,
    };
    for region in &regions {
        if region.iter().all(|line| line.trim().is_empty())
            || region.iter().any(|line| !is_portable(line))
        {
            return Err(UnionError::NotDeclarationOnly);
        }
    }
    let spans = marker_spans(content);
    if spans.len() != regions.len() {
        return Err(UnionError::UnbalancedMarkers {
            detail: format!("{} hunks but {} marker spans", regions.len(), spans.len()),
        });
    }
    let mut out = String::new();
    let mut cursor = 0usize;
    for (region, span) in regions.iter().zip(spans.iter()) {
        out.push_str(&content[cursor..span.body_start]);
        out.push_str(&union_block(region));
        cursor = span.end;
    }
    out.push_str(&content[cursor..]);
    Ok(out)
}

/// The error a malformed marker walk produces, named by what it found.
fn unbalanced(detail: &str) -> UnionError {
    UnionError::UnbalancedMarkers {
        detail: detail.to_string(),
    }
}

/// The byte span of one conflict region: the body between the markers and the
/// end of the closing marker line.
struct Span {
    body_start: usize,
    end: usize,
}

fn push_span(spans: &mut Vec<Span>, body_start: usize, end: usize) {
    spans.push(Span { body_start, end });
}

/// Byte spans of every conflict region, in order.
fn marker_spans(content: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut offset = 0usize;
    let mut open: Option<usize> = None;
    for raw in content.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        // `body_start` is the start of the `<<<<<<<` marker line, so the splice
        // that copies `content[cursor..body_start]` excludes the marker and the
        // union replaces the whole region (markers, both sides, separator) with
        // the canonical list.
        if line.starts_with(OPEN) {
            open = Some(offset);
        } else if line.starts_with(CLOSE) {
            if let Some(start) = open.take() {
                push_span(&mut spans, start, offset + raw.len());
            }
        }
        offset += raw.len();
    }
    spans
}

/// The sorted union of the declaration lines from both sides of one hunk.
fn union_block(lines: &[String]) -> String {
    // Attributes travel with the declaration that follows them, and a
    // declaration both sides added is emitted once.
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut pending: Vec<&str> = Vec::new();
    for line in lines {
        match classify_line(line) {
            Line::Attribute(attribute) => {
                pending.push(attribute.trim_end_matches(['\n', '\r']));
            }
            Line::Declaration(declaration) => {
                let (key, body) = entry(declaration.trim(), &pending);
                pending.clear();
                insert(&mut entries, key, body);
            }
            Line::Other(_) => {}
        }
    }
    debug_assert!(
        pending.is_empty(),
        "an attribute with no declaration is rejected before this point"
    );
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.iter().map(|(_, body)| body.as_str()).collect()
}

/// The (order key, emitted text) for one declaration, with any pending
/// attributes attached above it.
fn entry(declaration: &str, pending: &[&str]) -> (String, String) {
    let mut body = String::new();
    for attribute in pending {
        body.push_str(attribute);
        body.push('\n');
    }
    body.push_str(declaration);
    body.push('\n');
    (sort_key(declaration), body)
}

/// Insert a declaration into the union, replacing the text if the same
/// declaration was already seen on the other side of the hunk.
fn insert(entries: &mut Vec<(String, String)>, key: String, body: String) {
    match entries.iter().position(|(existing, _)| *existing == key) {
        Some(at) => entries[at].1 = body,
        None => entries.push((key, body)),
    }
}

/// The ordering key: the module path, so `alpha` precedes `beta` whatever
/// visibility qualifier or attribute the declaration carries. The full text is
/// the tie-break, so two declarations of one name cannot merge into each other.
fn sort_key(declaration: &str) -> String {
    let name = declared_module(declaration).unwrap_or(declaration);
    format!("{name}\u{0}{declaration}")
}

/// The declarations in `text` whose file is not in the merged tree.
///
/// A union cannot tell an addition from the other side's deletion, so the
/// filesystem decides: each name is offered to `exists`, and a declaration is
/// orphaned when neither `name.rs` nor `name/mod.rs` resolves.
pub fn orphaned<F>(text: &str, exists: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    declared_modules(text)
        .into_iter()
        .filter(|name| !exists(name))
        .collect()
}

/// Collect the lines inside every conflict hunk (both sides, in order).
///
/// `Ok(None)` when the file has no markers; `Err` when they are malformed,
/// which [`detect`] reports as [`Conflict::Mixed`] so a half-parsed file is
/// never mistaken for a safe one.
fn hunks(content: &str) -> Result<Option<Vec<Vec<String>>>, UnionError> {
    let mut regions: Vec<Vec<String>> = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for raw in content.lines() {
        if raw.starts_with(OPEN) || raw.starts_with(DIFF3) {
            match current.take() {
                Some(_) => return Err(unbalanced("a conflict marker opened inside an open hunk")),
                None => current = Some(Vec::new()),
            }
            continue;
        }
        if raw == SEP {
            continue;
        }
        if raw.starts_with(CLOSE) {
            match current.take() {
                Some(lines) => regions.push(lines),
                None => return Err(unbalanced("a conflict hunk closed without opening")),
            }
            continue;
        }
        if let Some(lines) = current.as_mut() {
            lines.push(raw.to_string());
        }
    }
    if current.is_some() {
        return Err(unbalanced("a conflict hunk is still open at end of file"));
    }
    match regions.is_empty() {
        true => Ok(None),
        false => Ok(Some(regions)),
    }
}
