//! Declaration-only conflict resolution for stranded patches (#3935).
//!
//! Four dispatches sat held as "does not apply" on one additive hunk in a
//! 27-line module-declaration file: `ours` held the existing `mod`
//! declarations and `theirs` added one more. The correct resolution was the
//! union of the declarations, but nothing said so — `does not apply` is a
//! fact about the patch, not about the conflict. Module-declaration files
//! (`mod.rs`, `lib.rs`) are the dominant conflict class in a
//! parallel-authorship Rust workspace: every issue that adds a module must
//! touch the same lines, no matter how unrelated the modules are. That is
//! not a decomposition failure; it is the module system meeting parallel
//! authorship, so the class is worth resolving precisely instead of holding.
//!
//! The resolver is pure: it takes the conflicted file content and returns a
//! resolved file, a refusal with a stated reason, or nothing to do. The
//! caller performs the git I/O and — per invariant 3 — the compile and test
//! I/O that a resolution must pass before it is offered.
//!
//! 1. **An additive conflict confined to module-declaration lines resolves
//!    automatically** ([`resolve`]): the resolution keeps `ours` verbatim
//!    and unions in the declarations `theirs` adds, in a stable order (ours
//!    order first, then theirs order).
//! 2. **Refusal is the default, not the exception** ([`Refusal`]). Doc
//!    comments, blank lines, `mod` declarations and `use` blocks are in
//!    scope; a single line of anything else, a removal visible in a diff3
//!    base section, or a hunk that does not close means hand it to a human.
//!    A resolver that cannot say no is a corruption engine — refusing is a
//!    success, not a failure.
//! 3. **A resolution is not offered before it compiles and passes the
//!    affected crate's tests** ([`ResolvedFile::mark_verified`],
//!    [`ResolvedFile::into_content`]): the type withholds its content until
//!    the caller has run the build and tests and marked the result.
//! 4. **A hold reports its shape, not just its existence**
//!    ([`hold_shape`]): the conflicted file, the hunk count and whether the
//!    conflict is declaration-only. `does not apply` cannot be routed;
//!    `1 conflicted file, 1 hunk, module declarations only` can.

use std::fmt;

/// Conflict marker prefixes. The start and end markers carry a label
/// (`<<<<<<< HEAD`, `>>>>>>> feat/x`); the separator does not.
const HUNK_START: &str = "<<<<<<<";
const HUNK_SEPARATOR: &str = "=======";
const HUNK_END: &str = ">>>>>>>";
const HUNK_BASE: &str = "|||||||";

/// The shape of a hold on one conflicted file.
///
/// `does not apply` is not enough to route a hold; the record must name the
/// conflicted file, the hunk count, and whether the conflict is
/// declaration-only (invariant 4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HoldShape {
    /// Repo-relative path of the conflicted file.
    pub file: String,
    /// Number of conflict hunks (`<<<<<<<` markers) in the file.
    pub hunks: usize,
    /// True when the resolver will resolve the conflict: every hunk, on
    /// every side, contains only in-scope lines (doc comments, blank lines,
    /// `mod` declarations, `use` blocks) and the file invokes no
    /// brace-bodied macro (whose expansion may declare modules the source
    /// does not list, so the declared-name set is not derivable).
    pub declaration_only: bool,
    /// Stated refusal reason when `declaration_only` is false.
    pub refusal: Option<String>,
}

impl fmt::Display for HoldShape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} hunk(s), {}",
            self.file,
            self.hunks,
            if self.declaration_only {
                "module declarations only"
            } else {
                "code present"
            },
        )?;
        if let Some(refusal) = &self.refusal {
            write!(f, " ({refusal})")?;
        }
        Ok(())
    }
}

/// Why the resolver declined a file. Refusing is a success, not a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    /// A line outside the resolver's scope (doc comments, blanks, `mod`
    /// declarations, `use` blocks).
    CodeBeyondDeclarations,
    /// A diff3 `|||||||` base section shows one side *removed* a
    /// declaration; the conflict is not purely additive.
    DeclarationRemoved,
    /// The same `mod` is declared with different text (visibility) on each
    /// side.
    ConflictingDeclarations,
    /// A `use` block never closed inside the conflict hunk.
    UnclosedUseBlock,
    /// A conflict marker has no matching pair, or appears outside one.
    MalformedHunk,
    /// The file invokes a brace-bodied macro whose expansion may declare `mod`
    /// items the source does not list, so the declared-name set is not
    /// derivable from the text and the resolver must refuse the file.
    UnenumerableDeclarations,
    /// The resolved file would declare the same `mod` more than once — the
    /// resolution is invalid and is refused at the point of production, before
    /// it is offered (invariant 3).
    DuplicateResolution,
}

impl fmt::Display for RefusalKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RefusalKind::CodeBeyondDeclarations => {
                "conflict contains code beyond module declarations"
            }
            RefusalKind::DeclarationRemoved => {
                "conflict removes a module declaration; not purely additive"
            }
            RefusalKind::ConflictingDeclarations => {
                "both sides declare the same module differently"
            }
            RefusalKind::UnclosedUseBlock => "use block does not close within the conflict hunk",
            RefusalKind::MalformedHunk => "malformed or unterminated conflict hunk",
            RefusalKind::UnenumerableDeclarations => {
                "a brace-bodied macro may declare modules the source does not list; \
                 the declared-name set is not derivable"
            }
            RefusalKind::DuplicateResolution => {
                "the resolution would declare the same module more than once"
            }
        })
    }
}

/// A refusal of an automatic resolution, with the stated reason that routes
/// it to a human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// Repo-relative path of the file that could not be resolved.
    pub file: String,
    /// The class of refusal.
    pub kind: RefusalKind,
    /// The offending line and content, when applicable.
    pub detail: String,
}

impl Refusal {
    fn code(file: &str, side: &str, line_no: usize, content: &str) -> Self {
        Self {
            file: file.to_string(),
            kind: RefusalKind::CodeBeyondDeclarations,
            detail: format!("line {line_no} ({side}): `{content}`"),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file, self.kind)?;
        if !self.detail.is_empty() {
            write!(f, " — {}", self.detail)?;
        }
        Ok(())
    }
}

/// The result of resolving one conflicted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// The content carried no conflict markers; there is nothing to resolve.
    NoConflict,
    /// The conflict was declaration-only and resolved to the union.
    Resolved(ResolvedFile),
    /// The conflict was not declaration-only; it routes to a human.
    Refused(Refusal),
}

/// A declaration-only resolution of one conflicted file, in the unverified
/// state.
///
/// Invariant 3: the content is withheld until the caller has applied it,
/// compiled the affected crate, and run its tests. A resolution that has not
/// been verified cannot be offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFile {
    /// Repo-relative path of the resolved file.
    pub file: String,
    /// `mod` names kept from `ours`, in ours order.
    pub kept: Vec<String>,
    /// Declarations unioned in from `theirs`, in theirs order: the `mod`
    /// name for module declarations, the statement text for `use` blocks.
    pub added: Vec<String>,
    content: String,
    verified: bool,
}

impl ResolvedFile {
    /// Whether the affected crate's compile and tests have been run and
    /// passed for this content.
    pub fn is_verified(&self) -> bool {
        self.verified
    }

    /// Mark the resolution verified: the caller has applied this content,
    /// compiled the affected crate, and run its tests, and both passed.
    pub fn mark_verified(mut self) -> Self {
        self.verified = true;
        self
    }

    /// The resolved file content. Only a verified resolution may be offered.
    pub fn into_content(self) -> Result<String, UnverifiedResolution> {
        if self.verified {
            Ok(self.content)
        } else {
            Err(UnverifiedResolution)
        }
    }

    /// `kept [config, models, proposals], added [correlate]` — the summary
    /// a merge note or log line should carry.
    pub fn summary(&self) -> String {
        format!(
            "kept [{}], added [{}]",
            self.kept.join(", "),
            self.added.join(", ")
        )
    }
}

impl fmt::Display for ResolvedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: merged, {}", self.file, self.summary())
    }
}

/// A resolution that has not been verified by compile and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnverifiedResolution;

impl fmt::Display for UnverifiedResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "resolution must be verified by compiling and running the affected \
             crate's tests before it is offered (#3935)",
        )
    }
}

impl std::error::Error for UnverifiedResolution {}

/// Report the shape of a hold on one conflicted file without resolving it
/// (invariant 4).
pub fn hold_shape(file: &str, content: &str) -> HoldShape {
    let marker_count = content
        .lines()
        .filter(|l| l.starts_with(HUNK_START))
        .count();
    match parse_hunks(content) {
        Err((kind, detail)) => HoldShape {
            file: file.to_string(),
            hunks: marker_count,
            declaration_only: false,
            refusal: Some(describe(kind, &detail)),
        },
        Ok(hunks) => {
            // A brace-bodied macro makes the declared-name set non-derivable,
            // so the file is not declaration-only (issue #4319). Only checked
            // when there is a conflict to resolve.
            let macro_refusal = hunks.first().and_then(|_| macro_refusal(file, content));
            if let Some(refusal) = macro_refusal {
                return HoldShape {
                    file: file.to_string(),
                    hunks: hunks.len(),
                    declaration_only: false,
                    refusal: Some(refusal.describe()),
                };
            }
            for hunk in &hunks {
                if let Err(refusal) = hunk_in_scope(file, hunk) {
                    return HoldShape {
                        file: file.to_string(),
                        hunks: hunks.len(),
                        declaration_only: false,
                        refusal: Some(refusal.describe()),
                    };
                }
            }
            HoldShape {
                file: file.to_string(),
                hunks: hunks.len(),
                declaration_only: true,
                refusal: None,
            }
        }
    }
}

impl Refusal {
    /// The stated reason without the file prefix (for [`HoldShape::refusal`]).
    fn describe(&self) -> String {
        describe(self.kind, &self.detail)
    }
}

fn describe(kind: RefusalKind, detail: &str) -> String {
    if detail.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} — {detail}")
    }
}

/// Resolve one conflicted file.
///
/// Returns [`ResolveOutcome::NoConflict`] when the content carries no
/// markers, [`ResolveOutcome::Resolved`] when every hunk on every side is
/// declaration-only (doc comments, blank lines, `mod` declarations, `use`
/// blocks) and the conflict is purely additive, and
/// [`ResolveOutcome::Refused`] with a stated reason otherwise.
pub fn resolve(file: &str, content: &str) -> ResolveOutcome {
    let hunks = match parse_hunks(content) {
        Ok(hunks) => hunks,
        Err((kind, detail)) => {
            return ResolveOutcome::Refused(Refusal {
                file: file.to_string(),
                kind,
                detail,
            })
        }
    };
    if hunks.is_empty() {
        return ResolveOutcome::NoConflict;
    }

    let lines: Vec<&str> = content.lines().collect();

    // Invariant (issue #4319): an automated resolver must be able to
    // enumerate what it is reasoning about, and refuse when it cannot. A
    // brace-bodied macro invocation (`commands! { … }`) may expand to `mod`
    // declarations the source does not list, so a textual union could emit a
    // name twice. Refuse and leave the conflict for a person.
    if let Some(refusal) = macro_refusal(file, content) {
        return ResolveOutcome::Refused(refusal);
    }

    // Names declared outside every conflict block (issue #4319): the
    // out-of-hunk lines are kept verbatim in the resolution, so a `theirs`
    // declaration of the same name is a duplicate the union must not
    // re-emit.
    let external_mods = external_mod_names(&lines, &hunks);

    let mut out: Vec<String> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();

    let mut i = 0usize;
    let mut h = 0usize;
    while i < lines.len() {
        if h < hunks.len() && i == hunks[h].start {
            let resolved = match resolve_hunk(file, &hunks[h], &external_mods) {
                Ok(r) => r,
                Err(refusal) => return ResolveOutcome::Refused(refusal),
            };
            out.extend(resolved.lines);
            kept.extend(resolved.kept);
            added.extend(resolved.added);
            i = hunks[h].end;
            h += 1;
        } else {
            out.push(lines[i].to_string());
            i += 1;
        }
    }

    // Invariant 3 (issue #4319): validate the output before reporting
    // success. A duplicate `mod` name in the resolved file cannot compile; the
    // cause is visible now, at production, rather than minutes later in a
    // compiler error attributed to the patch.
    if let Some(dup) = duplicate_mod_names(&out) {
        return ResolveOutcome::Refused(Refusal {
            file: file.to_string(),
            kind: RefusalKind::DuplicateResolution,
            detail: format!("the resolution would declare `mod {dup}` more than once"),
        });
    }

    ResolveOutcome::Resolved(ResolvedFile {
        file: file.to_string(),
        kept,
        added,
        content: out.join("\n") + "\n",
        verified: false,
    })
}

struct ResolvedHunk {
    /// The lines that replace the hunk region: `ours` verbatim, then the
    /// declarations unioned in from `theirs`.
    lines: Vec<String>,
    kept: Vec<String>,
    added: Vec<String>,
}

/// One parsed conflict hunk, with file-line bookkeeping for error messages.
struct Hunk {
    /// File line index (0-based) of the `<<<<<<<` marker.
    start: usize,
    /// File line index (0-based) just past the `>>>>>>>` marker.
    end: usize,
    /// File line index (0-based) of the first `ours` line.
    ours_first: usize,
    /// File line index (0-based) of the first base line, if diff3.
    base_first: Option<usize>,
    /// File line index (0-based) of the first `theirs` line.
    theirs_first: usize,
    ours: Vec<String>,
    base: Option<Vec<String>>,
    theirs: Vec<String>,
}

enum HunkSide {
    Ours,
    Base,
    Theirs,
}

/// Split content into conflict hunks.
///
/// Errors with [`RefusalKind::MalformedHunk`] when a hunk is unterminated or
/// a marker appears outside a hunk.
fn parse_hunks(content: &str) -> Result<Vec<Hunk>, (RefusalKind, String)> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut side: Option<HunkSide> = None;

    for (idx, line) in content.lines().enumerate() {
        match side {
            None => {
                if line.starts_with(HUNK_START) {
                    side = Some(HunkSide::Ours);
                    hunks.push(Hunk {
                        start: idx,
                        end: 0,
                        ours_first: idx + 1,
                        base_first: None,
                        theirs_first: 0,
                        ours: Vec::new(),
                        base: None,
                        theirs: Vec::new(),
                    });
                } else if line == HUNK_SEPARATOR
                    || line.starts_with(HUNK_END)
                    || line.starts_with(HUNK_BASE)
                {
                    return Err((
                        RefusalKind::MalformedHunk,
                        format!("line {}: stray conflict marker outside a hunk", idx + 1),
                    ));
                }
            }
            Some(HunkSide::Ours) => {
                if line == HUNK_SEPARATOR {
                    let hunk = hunks.last_mut().expect("ours implies an open hunk");
                    hunk.theirs_first = idx + 1;
                    side = Some(HunkSide::Theirs);
                } else if line.starts_with(HUNK_BASE) {
                    let hunk = hunks.last_mut().expect("ours implies an open hunk");
                    hunk.base_first = Some(idx + 1);
                    hunk.base = Some(Vec::new());
                    side = Some(HunkSide::Base);
                } else {
                    hunks
                        .last_mut()
                        .expect("ours implies an open hunk")
                        .ours
                        .push(line.to_string());
                }
            }
            Some(HunkSide::Base) => {
                if line == HUNK_SEPARATOR {
                    let hunk = hunks.last_mut().expect("base implies an open hunk");
                    hunk.theirs_first = idx + 1;
                    side = Some(HunkSide::Theirs);
                } else {
                    hunks
                        .last_mut()
                        .expect("base implies an open hunk")
                        .base
                        .as_mut()
                        .expect("base section collected on entry")
                        .push(line.to_string());
                }
            }
            Some(HunkSide::Theirs) => {
                if line.starts_with(HUNK_END) {
                    let hunk = hunks.last_mut().expect("theirs implies an open hunk");
                    hunk.end = idx + 1;
                    side = None;
                } else {
                    hunks
                        .last_mut()
                        .expect("theirs implies an open hunk")
                        .theirs
                        .push(line.to_string());
                }
            }
        }
    }

    if side.is_some() {
        return Err((
            RefusalKind::MalformedHunk,
            "unterminated conflict hunk".to_string(),
        ));
    }
    Ok(hunks)
}

/// One in-scope line of a conflict side.
enum LineKind {
    /// Blank line.
    Blank,
    /// Doc comment line (`///` or `//!`).
    Doc,
    /// `mod` declaration: `pub mod name;` (any visibility).
    Mod { name: String, text: String },
    /// `use` statement, normalized to a single line.
    Use { text: String },
}

impl LineKind {
    /// Identity for deduplication: the module name for `mod`, the
    /// normalized statement text for `use`.
    fn key(&self) -> Option<String> {
        match self {
            LineKind::Mod { name, .. } => Some(format!("mod:{name}")),
            LineKind::Use { text } => Some(format!("use:{text}")),
            LineKind::Blank | LineKind::Doc => None,
        }
    }

    /// Human-readable identity for refusal details.
    fn label(&self) -> String {
        match self {
            LineKind::Mod { name, .. } => format!("`mod {name}`"),
            LineKind::Use { text } => format!("`{text}`"),
            LineKind::Blank => "blank line".to_string(),
            LineKind::Doc => "doc comment".to_string(),
        }
    }
}

/// Classify one side of a hunk; every line must be in scope.
fn classify_side(
    file: &str,
    side: &str,
    lines: &[String],
    first_line: usize,
) -> Result<Vec<LineKind>, Refusal> {
    let mut out: Vec<LineKind> = Vec::new();
    let mut pending_use: Option<String> = None;

    for (j, line) in lines.iter().enumerate() {
        let line_no = first_line + j + 1;
        let t = line.trim();

        if let Some(acc) = pending_use.take() {
            if t.ends_with(';') {
                out.push(LineKind::Use {
                    text: normalize(&format!("{acc} {t}")),
                });
            } else {
                pending_use = Some(format!("{acc} {t}"));
            }
            continue;
        }

        if t.is_empty() {
            out.push(LineKind::Blank);
        } else if t.starts_with("///") || t.starts_with("//!") {
            out.push(LineKind::Doc);
        } else if let Some(name) = parse_mod(t) {
            out.push(LineKind::Mod {
                name,
                text: t.to_string(),
            });
        } else if use_start(t) {
            if t.ends_with(';') {
                out.push(LineKind::Use { text: normalize(t) });
            } else {
                pending_use = Some(t.to_string());
            }
        } else {
            return Err(Refusal::code(file, side, line_no, t));
        }
    }

    if pending_use.is_some() {
        return Err(Refusal {
            file: file.to_string(),
            kind: RefusalKind::UnclosedUseBlock,
            detail: format!("line {} ({side})", first_line + 1),
        });
    }
    Ok(out)
}

/// Resolve one hunk to its replacement lines plus the kept/added report.
fn resolve_hunk(
    file: &str,
    hunk: &Hunk,
    external_mods: &[String],
) -> Result<ResolvedHunk, Refusal> {
    let ours = classify_side(file, "ours", &hunk.ours, hunk.ours_first)?;
    let theirs = classify_side(file, "theirs", &hunk.theirs, hunk.theirs_first)?;
    let base = match &hunk.base {
        Some(lines) => Some(classify_side(
            file,
            "base",
            lines,
            hunk.base_first.expect("base_first set when base collected"),
        )?),
        None => None,
    };

    if let Some(base) = &base {
        // A diff3 base section is ground truth for what each side removed;
        // a removal is not additive, so it refuses.
        let ours_keys: Vec<String> = ours.iter().filter_map(|d| d.key()).collect();
        let theirs_keys: Vec<String> = theirs.iter().filter_map(|d| d.key()).collect();
        for decl in base {
            let Some(key) = decl.key() else {
                continue;
            };
            if !ours_keys.contains(&key) {
                return Err(Refusal {
                    file: file.to_string(),
                    kind: RefusalKind::DeclarationRemoved,
                    detail: format!("ours removes {}", decl.label()),
                });
            }
            if !theirs_keys.contains(&key) {
                return Err(Refusal {
                    file: file.to_string(),
                    kind: RefusalKind::DeclarationRemoved,
                    detail: format!("theirs removes {}", decl.label()),
                });
            }
        }
    }

    // Same module, different text on each side (a visibility difference):
    // the resolver will not guess which wins.
    for ours_decl in ours.iter() {
        if let LineKind::Mod { name, text } = ours_decl {
            if let Some(LineKind::Mod {
                text: theirs_text, ..
            }) = theirs
                .iter()
                .find(|d| matches!(d, LineKind::Mod { name: n, .. } if n == name))
            {
                if theirs_text != text {
                    return Err(Refusal {
                        file: file.to_string(),
                        kind: RefusalKind::ConflictingDeclarations,
                        detail: format!("`mod {name}`: ours `{text}`, theirs `{theirs_text}`"),
                    });
                }
            }
        }
    }

    let ours_mods: Vec<&str> = ours
        .iter()
        .filter_map(|d| {
            if let LineKind::Mod { name, .. } = d {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect();
    let ours_uses: Vec<&str> = ours
        .iter()
        .filter_map(|d| {
            if let LineKind::Use { text } = d {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect();

    let mut added: Vec<String> = Vec::new();
    let mut lines: Vec<String> = hunk.ours.clone();

    for decl in &theirs {
        match decl {
            LineKind::Mod { name, text }
                if !ours_mods.contains(&name.as_str())
                    && !external_mods.iter().any(|m| m.as_str() == name.as_str()) =>
            {
                added.push(name.clone());
                lines.push(text.clone());
            }
            LineKind::Use { text } if !ours_uses.contains(&text.as_str()) => {
                added.push(text.clone());
                lines.push(text.clone());
            }
            _ => {}
        }
    }
    // `kept` reports the modules kept from ours, in ours order.
    let kept: Vec<String> = ours
        .iter()
        .filter_map(|d| {
            if let LineKind::Mod { name, .. } = d {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    Ok(ResolvedHunk { lines, kept, added })
}

/// Check that one hunk is declaration-only on every side, without building
/// the resolution.
fn hunk_in_scope(file: &str, hunk: &Hunk) -> Result<(), Refusal> {
    classify_side(file, "ours", &hunk.ours, hunk.ours_first)?;
    if let Some(lines) = &hunk.base {
        classify_side(
            file,
            "base",
            lines,
            hunk.base_first.expect("base_first set when base collected"),
        )?;
    }
    classify_side(file, "theirs", &hunk.theirs, hunk.theirs_first)?;
    Ok(())
}

/// Parse `pub mod name;` (any visibility) and return the module name.
fn parse_mod(t: &str) -> Option<String> {
    let rest = strip_visibility(t)?;
    let after = rest.strip_prefix("mod ")?;
    let after = after.trim_start();
    let semi = after.find(';')?;
    let name = after[..semi].trim();
    if name.is_empty() || !is_ident(name) || !after[semi + 1..].trim().is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Strip a leading `pub`, `pub(crate)`, `pub(super)`, `pub(self)`.
fn strip_visibility(t: &str) -> Option<&str> {
    if let Some(rest) = t.strip_prefix("pub ") {
        Some(rest)
    } else if let Some(paren) = t.strip_prefix("pub(") {
        let close = paren.find(')')?;
        Some(paren[close + 1..].trim_start())
    } else {
        Some(t)
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One identifier character (ASCII alphanumeric or `_`).
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// One character that may appear in a macro invocation name: an identifier
/// character or a path separator (`::` / `.`), so `foo::bar!` reports the
/// full `foo::bar`.
fn is_macro_name_char(c: char) -> bool {
    is_ident_char(c) || c == ':' || c == '.'
}

/// The module names declared on the lines OUTSIDE every conflict hunk, in
/// file order (issue #4319). These lines are kept verbatim in the
/// resolution, so a `theirs` declaration of the same name is a duplicate the
/// union must not re-emit.
fn external_mod_names(lines: &[&str], hunks: &[Hunk]) -> Vec<String> {
    let mut in_hunk: Vec<bool> = vec![false; lines.len()];
    for hunk in hunks {
        for idx in hunk.start..hunk.end {
            if idx < in_hunk.len() {
                in_hunk[idx] = true;
            }
        }
    }
    let mut names: Vec<String> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if in_hunk[idx] {
            continue;
        }
        if let Some(name) = parse_mod(line.trim()) {
            if !names.iter().any(|existing| existing == &name) {
                names.push(name);
            }
        }
    }
    names
}

/// Locate the first brace-bodied macro invocation in the content, skipping
/// comments and string literals, and return its 1-based line number and the
/// invocation name (issue #4319). A brace-bodied macro is `<name>!` followed
/// (after optional whitespace) by `{`; a `macro_rules! name {` *definition*
/// is not matched, because the `!` there is followed by the name, not `{`.
fn find_brace_bodied_macro(content: &str) -> Option<(usize, String)> {
    let masked = mask_comments_and_strings(content);
    let chars: Vec<char> = masked.chars().collect();
    let mut line_no = 1usize;
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '!' && i > 0 && is_ident_char(chars[i - 1]) {
            let mut j = i + 1;
            let mut j_line = line_no;
            while j < chars.len() && chars[j].is_whitespace() {
                if chars[j] == '\n' {
                    j_line += 1;
                }
                j += 1;
            }
            if j < chars.len() && chars[j] == '{' {
                let mut name_start = i - 1;
                while name_start > 0 && is_macro_name_char(chars[name_start - 1]) {
                    name_start -= 1;
                }
                let name: String = chars[name_start..i].iter().collect();
                return Some((j_line, name));
            }
        }
        if chars[i] == '\n' {
            line_no += 1;
        }
        i += 1;
    }
    None
}

/// The first `mod` name declared more than once in the resolved lines, if any
/// (invariant 3): a duplicate would emit one name twice, which cannot compile,
/// so the resolution is refused at the point of production rather than
/// reported as success and blamed on the patch later.
fn duplicate_mod_names(lines: &[String]) -> Option<String> {
    let mut seen: Vec<String> = Vec::new();
    for line in lines {
        let Some(name) = parse_mod(line.trim()) else {
            continue;
        };
        if seen.iter().any(|existing| existing == &name) {
            return Some(name);
        }
        seen.push(name);
    }
    None
}

/// A refusal for a file that invokes a brace-bodied macro (issue #4319),
/// naming the macro and its line: the declared-name set is not derivable from
/// the source, so the resolver refuses the file. `None` when the file invokes
/// no such macro.
fn macro_refusal(file: &str, content: &str) -> Option<Refusal> {
    find_brace_bodied_macro(content).map(|(line_no, macro_name)| Refusal {
        file: file.to_string(),
        kind: RefusalKind::UnenumerableDeclarations,
        detail: format!(
            "line {line_no}: `{macro_name}!` may declare modules the source does not list"
        ),
    })
}

/// Blank out comment and string-literal interiors, preserving the original
/// length and line structure, so the brace-bodied-macro scan does not fire on
/// a `!` inside a doc comment or a help string.
fn mask_comments_and_strings(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0usize;
    let mut in_block_comment = false;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_block_comment {
            if c == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                out.push(' ');
                out.push(' ');
                in_block_comment = false;
                i += 2;
            } else {
                out.push(if c == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        if in_string {
            if c == '\\' && i + 1 < chars.len() {
                out.push(' ');
                out.push(if chars[i + 1] == '\n' { '\n' } else { ' ' });
                i += 2;
            } else if c == '"' {
                out.push(' ');
                in_string = false;
                i += 1;
            } else {
                out.push(if c == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            out.push(' ');
            out.push(' ');
            in_block_comment = true;
            i += 2;
            continue;
        }
        if c == '"' {
            out.push(' ');
            in_string = true;
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out.into_iter().collect()
}

/// Whether a trimmed line starts a `use` statement (possibly multi-line).
fn use_start(t: &str) -> bool {
    if t == "use" || t.starts_with("use ") {
        return true;
    }
    if t == "pub use" || t.starts_with("pub use ") {
        return true;
    }
    if let Some(paren) = t.strip_prefix("pub(") {
        if let Some(close) = paren.find(')') {
            let after = paren[close + 1..].trim_start();
            return after == "use" || after.starts_with("use ");
        }
    }
    false
}

/// Collapse internal whitespace so equivalent statements deduplicate.
fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "crates/autospec-core/src/insights/mod.rs";

    fn conflicted(ours: &[&str], theirs: &[&str]) -> String {
        let mut s = String::from("<<<<<<< HEAD\n");
        s.push_str(&ours.join("\n"));
        s.push_str("\n=======\n");
        s.push_str(&theirs.join("\n"));
        s.push_str("\n>>>>>>> feat/insights\n");
        s
    }

    fn resolved(content: &str) -> ResolvedFile {
        match resolve(FILE, content) {
            ResolveOutcome::Resolved(r) => r,
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[test]
    fn declaration_only_conflict_resolves_to_union_in_stable_order() {
        let content = format!(
            "//! Continuous Improvement Engine subsystem.\n\n{}",
            conflicted(
                &["pub mod config;", "pub mod models;", "pub mod proposals;"],
                &["pub mod correlate;"],
            )
        );
        let r = resolved(&content);
        assert_eq!(r.kept, vec!["config", "models", "proposals"]);
        assert_eq!(r.added, vec!["correlate"]);
        assert!(!r.is_verified());
        let content = r.mark_verified().into_content().expect("verified");
        assert_eq!(
            content,
            "//! Continuous Improvement Engine subsystem.\n\n\
             pub mod config;\n\
             pub mod models;\n\
             pub mod proposals;\n\
             pub mod correlate;\n"
        );
    }

    #[test]
    fn a_single_line_of_real_code_refuses_with_a_stated_reason() {
        let content = conflicted(
            &["pub mod config;", "pub mod models;"],
            &["pub mod correlate;", "pub fn stray() -> u32 { 1 }"],
        );
        match resolve(FILE, &content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::CodeBeyondDeclarations);
                assert!(
                    refusal.to_string().contains("pub fn stray() -> u32 { 1 }"),
                    "reason names the offending line: {}",
                    refusal
                );
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn use_blocks_are_in_scope_single_and_multi_line() {
        let content = conflicted(
            &["pub use models::{Config, Proposal};"],
            &[
                "pub use models::{Correlate, Proposal};",
                "use storage::{",
                "    reader,",
                "    writer,",
                "};",
            ],
        );
        let r = resolved(&content);
        assert_eq!(
            r.added,
            vec![
                "pub use models::{Correlate, Proposal};",
                "use storage::{ reader, writer, };",
            ]
        );
    }

    #[test]
    fn doc_comments_and_blank_lines_are_in_scope() {
        // In-scope on both sides; the resolution keeps ours verbatim and
        // unions in only the declarations theirs adds.
        let content = conflicted(
            &["//! ingest stage", "", "pub mod ingest;"],
            &["//! pattern stage", "", "pub mod pattern;"],
        );
        let r = resolved(&content);
        assert_eq!(r.added, vec!["pattern"]);
        let content = r.mark_verified().into_content().unwrap();
        assert_eq!(
            content,
            "//! ingest stage\n\npub mod ingest;\npub mod pattern;\n"
        );
    }

    #[test]
    fn diff3_base_shows_ours_removed_a_declaration_so_refuse() {
        // Base held `old`; ours dropped it. Not purely additive -> refuse.
        let content = "<<<<<<< HEAD\n\
                       pub mod a;\n\
                       ||||||| merged common ancestors\n\
                       pub mod a;\n\
                       pub mod old;\n\
                       =======\n\
                       pub mod a;\n\
                       pub mod b;\n\
                       >>>>>>> feat/x\n";
        match resolve(FILE, content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::DeclarationRemoved);
                assert!(refusal.detail.contains("ours removes `mod old`"));
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn diff3_purely_additive_resolves() {
        let content = "<<<<<<< HEAD\n\
                       pub mod a;\n\
                       ||||||| merged common ancestors\n\
                       pub mod a;\n\
                       =======\n\
                       pub mod a;\n\
                       pub mod b;\n\
                       >>>>>>> feat/x\n";
        let r = resolved(content);
        assert_eq!(r.added, vec!["b"]);
    }

    #[test]
    fn same_mod_different_visibility_refuses() {
        let content = conflicted(&["pub mod a;"], &["mod a;"]);
        match resolve(FILE, &content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::ConflictingDeclarations);
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn unterminated_hunk_refuses() {
        let content = "<<<<<<< HEAD\npub mod a;\n=======\npub mod b;";
        match resolve(FILE, content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::MalformedHunk);
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn unclosed_use_block_refuses() {
        let content = conflicted(&["pub mod a;"], &["use storage::{", "    reader,"]);
        match resolve(FILE, &content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::UnclosedUseBlock);
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn hold_shape_reports_file_hunks_and_declaration_only() {
        let content = conflicted(&["pub mod a;"], &["pub mod a;", "pub mod b;"]);
        let shape = hold_shape(FILE, &content);
        assert!(shape.declaration_only);
        assert_eq!(shape.hunks, 1);
        assert!(shape.refusal.is_none());
        assert_eq!(
            shape.to_string(),
            format!("{FILE}: 1 hunk(s), module declarations only")
        );

        let shape = hold_shape(
            FILE,
            &conflicted(&["pub mod a;"], &["pub mod a;", "fn main() {}"]),
        );
        assert!(!shape.declaration_only);
        assert_eq!(shape.hunks, 1);
        let refusal = shape.refusal.expect("stated reason");
        assert!(refusal.contains("code beyond module declarations"));
        assert!(refusal.contains("fn main() {}"));
    }

    #[test]
    fn clean_content_is_no_conflict() {
        assert_eq!(resolve(FILE, "pub mod a;\n"), ResolveOutcome::NoConflict);
        let shape = hold_shape(FILE, "pub mod a;\n");
        assert_eq!(shape.hunks, 0);
        assert!(shape.declaration_only);
    }

    #[test]
    fn code_outside_the_hunk_passes_through_verbatim() {
        let content = "//! lib\n\n\
                       pub fn engine() -> u32 {\n    42\n}\n\n\
                       <<<<<<< HEAD\n\
                       pub mod a;\n\
                       =======\n\
                       pub mod a;\n\
                       pub mod b;\n\
                       >>>>>>> feat/x\n\
                       \n\
                       #[cfg(test)]\nmod tests {}\n";
        let r = resolved(content);
        let content = r.mark_verified().into_content().unwrap();
        assert!(content.contains("pub fn engine() -> u32 {\n    42\n}"));
        assert!(content.contains("pub mod b;"));
        assert!(!content.contains("<<<<<<<"));
    }

    #[test]
    fn unverified_resolution_cannot_be_offered() {
        let r = resolved(&conflicted(&["pub mod a;"], &["pub mod a;", "pub mod b;"]));
        assert!(matches!(r.into_content(), Err(UnverifiedResolution)));
    }

    #[test]
    fn multiple_hunks_each_resolve_with_a_combined_report() {
        let content = "<<<<<<< HEAD\npub mod a;\n=======\npub mod a;\npub mod b;\n>>>>>>> feat/x\n\
                       //! mid\n\
                       <<<<<<< HEAD\npub mod c;\n=======\npub mod c;\npub mod d;\n>>>>>>> feat/x\n";
        let r = resolved(content);
        assert_eq!(r.kept, vec!["a", "c"]);
        assert_eq!(r.added, vec!["b", "d"]);
    }

    #[test]
    fn stray_separator_outside_a_hunk_refuses() {
        let content = "pub mod a;\n=======\npub mod b;\n";
        match resolve(FILE, content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::MalformedHunk);
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn a_brace_bodied_macro_refuses_the_whole_file() {
        // issue #4319: `commands! { … }` expands to `pub mod` declarations
        // the source does not list, so the declared-name set is not
        // derivable and the resolver refuses the file instead of unioning
        // it — even when the hunk itself is declaration-only.
        let content = "
//! commands
pub mod dispatch_spec;

<<<<<<< HEAD
pub mod managed_project;
=======
pub mod managed_project;
pub mod new_module;
>>>>>>> feat/add-module

commands! {
    init => \"Initialize\", diagnostic;
    aar => \"Inspect\", direct;
}
";
        match resolve(FILE, content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::UnenumerableDeclarations);
                assert!(
                    refusal.detail.contains("commands"),
                    "the reason names the macro: {}",
                    refusal.detail
                );
            }
            other => panic!("expected Refused, got {other:?}"),
        }
        // hold_shape agrees the conflict is not declaration-only.
        let shape = hold_shape(FILE, content);
        assert!(!shape.declaration_only, "{shape}");
        assert_eq!(shape.hunks, 1);
        let refusal = shape.refusal.expect("stated reason");
        assert!(refusal.contains("brace-bodied macro"), "{refusal}");
    }

    #[test]
    fn a_macro_mentioned_in_a_doc_comment_does_not_refuse() {
        // The `!` is inside a doc comment, not a real invocation. The scan
        // must not fire on comment text, so an ordinary declaration-only
        // conflict still unions.
        let content = "
//! Use `commands! { … }` to declare modules.
<<<<<<< HEAD
pub mod a;
=======
pub mod a;
pub mod b;
>>>>>>> feat/x
";
        let r = resolved(content);
        assert_eq!(r.kept, vec!["a"]);
        assert_eq!(r.added, vec!["b"]);
    }

    #[test]
    fn a_macro_definition_alone_does_not_refuse() {
        // `macro_rules! name {` is a definition, not an invocation: its `!`
        // is followed by the name, not `{`. Only an invocation refuses.
        let content = "
macro_rules! declare {
    ($($m:ident;)*) => { $($m)*; };
}
<<<<<<< HEAD
pub mod a;
=======
pub mod a;
pub mod b;
>>>>>>> feat/x
";
        let r = resolved(content);
        assert_eq!(r.kept, vec!["a"]);
        assert_eq!(r.added, vec!["b"]);
    }

    #[test]
    fn a_resolution_that_would_duplicate_a_mod_is_refused() {
        // Invariant 3 (issue #4319): the resolver validates its own output.
        // Two separate hunks each add `pub mod x;`; `x` is in neither hunk's
        // `ours` nor outside any hunk, so the union would emit `mod x` twice.
        // The resolver refuses at the point of production instead of reporting
        // success and letting a compiler blame the patch.
        let content = "<<<<<<< HEAD\npub mod a;\n=======\npub mod a;\npub mod x;\n>>>>>>> feat/1\n\
                       //! mid\n\
                       <<<<<<< HEAD\npub mod b;\n=======\npub mod b;\npub mod x;\n>>>>>>> feat/2\n";
        match resolve(FILE, content) {
            ResolveOutcome::Refused(refusal) => {
                assert_eq!(refusal.kind, RefusalKind::DuplicateResolution);
                assert!(refusal.detail.contains("x"), "{refusal}");
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn a_theirs_addition_declared_outside_the_hunk_is_not_reemitted() {
        // issue #4319: `main` declares `pub mod dispatch_spec;` outside the
        // conflict block (a "Helper modules" section); the patch adds the
        // same declaration to the top of the file inside the hunk. The union
        // keeps the out-of-hunk declaration verbatim and must NOT re-emit the
        // hunk-side copy, or the name is declared twice.
        let content = "
//! header
<<<<<<< HEAD
pub mod alpha;
pub mod beta;
=======
pub mod alpha;
pub mod beta;
pub mod dispatch_spec;
>>>>>>> feat/patch

// Helper modules
pub mod dispatch_spec;
";
        let r = resolved(content);
        assert_eq!(r.kept, vec!["alpha", "beta"]);
        assert!(
            r.added.is_empty(),
            "no re-emission of an already-declared name: {:?}",
            r.added
        );
        let content = r.mark_verified().into_content().unwrap();
        assert_eq!(
            content.matches("pub mod dispatch_spec;").count(),
            1,
            "exactly one declaration survives:\n{content}"
        );
        assert!(content.contains("pub mod beta;"));
        assert!(!content.contains("<<<<<<<"));
    }
}
