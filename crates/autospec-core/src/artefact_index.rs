//! Accumulating artefacts need a generated index and a size budget (issue
//! #4338).
//!
//! The incident: one reference page, hand-written and appended to for six
//! months, grew into something no model could read. 328 sections, 613
//! rule-boxes, 842 KB, 120,082 words. A model cannot hold it in context
//! (~156,000 tokens as text, ~210,000 as raw HTML — far above the context
//! window of most models it exists to inform), and the navigation the page
//! offers is one `<h1>` and zero anchors, so a consumer cannot address a
//! section either. The page had become un-consultable in exactly the two
//! ways a reference can fail: too big to ingest, too flat to navigate.
//!
//! Two sections sat 200 sections apart with near-identical titles, and
//! nobody could see it because no structure was ever generated from the
//! document — the "index" was whatever the author had typed.
//!
//! The general lesson: **an accumulating artefact needs a generated index
//! and a size budget.** An append-only document accretes structure nobody
//! declared and grows past any limit nobody set, because nothing derives
//! the current shape from the content and nothing measures the result.
//!
//! Five invariants, each checkable:
//!
//! 1. **Any entry added to an accumulating artefact adds an index entry in
//!    the same change, and the index is required from the first entry** —
//!    [`index_required`], [`IndexStatus::Missing`]. "From the first entry"
//!    is the point: an artefact that starts without an index never gets
//!    one, because the index is invisible as a need until the artefact is
//!    already un-navigable.
//! 2. **Any artefact intended as model input declares a size budget and
//!    fails loudly when it exceeds it.** Emit the token estimate on every
//!    write; refuse or split past the threshold — [`SizeBudget`],
//!    [`WriteAction`]. This is the invariant to mechanise first.
//! 3. **The index is generated from the document's structure — headings to
//!    anchored links, in document order — never hand-maintained beside it**
//!    — [`DocumentIndex::generate`], [`render`], [`index_status`]. A
//!    hand-maintained index drifts the moment a section is renamed; the
//!    generated one cannot.
//! 4. **Verify the index by count, not by eye: heading count == link count
//!    == unique id count.** A slug collision silently points two entries
//!    at one section — [`DocumentIndex::verify`], [`SlugCollision`].
//! 5. Plus a header stating the page is a reference to consult selectively,
//!    not a document to ingest — [`REFERENCE_HEADER`],
//!    [`ensure_reference_header`].
//!
//! The measured numbers are the calibration for [`estimate_tokens`]:
//! 120,082 words rendered ~156,000 tokens as text (≈1.3 tokens/word) and
//! ~210,000 tokens as raw HTML (≈1.75 tokens/word). The default budget
//! ([`DEFAULT_TOKEN_BUDGET`]) is the smallest context window the fleet
//! declares (32k), because an artefact that only fits the largest card
//! cannot be handed to most of the models it exists to inform.

use std::collections::BTreeMap;

use crate::accumulating_artefact_token_ratio::{TOKENS_PER_HTML_WORD_DEN, TOKENS_PER_HTML_WORD_NUM, TOKENS_PER_TEXT_WORD_DEN, TOKENS_PER_TEXT_WORD_NUM};

// ── Invariant 2: the size budget ────────────────────────────────────────────

/// The default token budget for an artefact intended as model input: the
/// smallest context window the fleet declares (32k). An artefact above it
/// fits only the largest card and cannot be handed to most of the models it
/// exists to inform.
pub const DEFAULT_TOKEN_BUDGET: usize = 32_000;

/// Text tokens per word, measured from the incident: 120,082 words rendered
/// ~156,000 tokens (156,000 / 120,082 ≈ 1.299).
pub const TOKENS_PER_TEXT_WORD: [usize; 2] = [13, 10];

/// Raw-HTML tokens per word, measured from the same document rendered as
/// HTML: 120,082 words rendered ~210,000 tokens (210,000 / 120,082 ≈ 1.749).
/// Markup is what the extra ratio is: tags tokenize, prose does not.
pub const TOKENS_PER_HTML_WORD: [usize; 2] = [7, 4];

/// Which rendering the estimate is for. The same words cost differently:
/// the incident page measured ~156,000 tokens as text and ~210,000 as raw
/// HTML, and a budget checked against the wrong one under-reports by a
/// third.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenFormat {
    /// Markdown / plain text (≈1.3 tokens per word).
    #[default]
    Text,
    /// Raw HTML markup (≈1.75 tokens per word).
    Html,
}

impl TokenFormat {
    pub fn ratio(&self) -> [usize; 2] {
        match self {
            TokenFormat::Text => TOKENS_PER_TEXT_WORD,
            TokenFormat::Html => TOKENS_PER_HTML_WORD,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            TokenFormat::Text => "text",
            TokenFormat::Html => "html",
        }
    }
}

/// Estimate the token count of `content` for `format`.
///
/// A word-count heuristic with the incident's measured ratio, not a tokenizer:
/// the budget question is "does this fit a context window", which a ±10%
/// estimate answers, and no tokenizer dependency is warranted for it. The
/// estimate is always emitted beside the verdict ([`WriteAction::line`]) so
/// a reader sees the number a refusal was decided on.
pub fn estimate_tokens_in(content: &str, format: TokenFormat) -> usize {
    let words = content
        .split_whitespace()
        .count()
        .saturating_mul(format.ratio()[0]);
    words / format.ratio()[1]
}

/// The default (text) estimate.
pub fn estimate_tokens(content: &str) -> usize {
    estimate_tokens_in(content, TokenFormat::Text)
}

/// A declared size budget for one artefact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeBudget {
    /// The maximum estimated tokens the artefact may have on write.
    pub max_tokens: usize,
    /// The rendering the estimate applies to.
    pub format: TokenFormat,
}

impl Default for SizeBudget {
    fn default() -> Self {
        Self::new(DEFAULT_TOKEN_BUDGET, TokenFormat::Text).expect("default budget is non-zero")
    }
}

impl SizeBudget {
    /// A budget of zero is not a budget, it is a refusal to have one: it
    /// rejects every write including the empty artefact, which presents as
    /// a tool that never writes rather than a policy. Refused at
    /// construction.
    pub fn new(max_tokens: usize, format: TokenFormat) -> Option<Self> {
        (max_tokens > 0).then_some(Self { max_tokens, format })
    }

    pub fn estimate(&self, content: &str) -> usize {
        estimate_tokens_in(content, self.format)
    }

    /// Invariant 2: the artefact fails loudly when it exceeds the budget.
    pub fn verdict(&self, content: &str) -> BudgetVerdict {
        let tokens = self.estimate(content);
        if tokens <= self.max_tokens {
            BudgetVerdict::Within {
                tokens,
                max: self.max_tokens,
                format: self.format,
            }
        } else {
            BudgetVerdict::Exceeded {
                tokens,
                max: self.max_tokens,
                over_by: tokens - self.max_tokens,
                format: self.format,
            }
        }
    }
}

/// The outcome of one budget check. Both arms carry the token estimate: the
/// estimate is emitted on *every* write, not only on a refusal, so the
/// growth curve is visible in the log before the wall arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetVerdict {
    Within {
        tokens: usize,
        max: usize,
        format: TokenFormat,
    },
    Exceeded {
        tokens: usize,
        max: usize,
        over_by: usize,
        format: TokenFormat,
    },
}

impl BudgetVerdict {
    pub fn is_within(&self) -> bool {
        matches!(self, BudgetVerdict::Within { .. })
    }

    pub fn tokens(&self) -> usize {
        match self {
            BudgetVerdict::Within { tokens, .. } | BudgetVerdict::Exceeded { tokens, .. } => *tokens,
        }
    }

    /// The line emitted on every write.
    pub fn line(&self) -> String {
        match self {
            BudgetVerdict::Within { tokens, max, format } => format!(
                "size: ~{tokens} tokens {format} (budget {max}) — within"
            ),
            BudgetVerdict::Exceeded {
                tokens,
                max,
                over_by,
                format,
            } => format!(
                "size: ~{tokens} tokens {format} (budget {max}) — OVER by {over_by}"
            ),
        }
    }
}

// ── Invariants 1 & 3: the generated index ───────────────────────────────────

/// One ATX heading in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1 (`#`) through 6 (`######`).
    pub level: u8,
    /// Heading text with the `#` markers and trailing hashes stripped.
    pub title: String,
    /// 1-based source line.
    pub line: usize,
}

/// The marker pair the generated index lives between. The markers are HTML
/// comments so they render invisibly and the block is machine-replaceable
/// on every write (invariant 3: generated, never hand-maintained).
pub const INDEX_BEGIN: &str = "<!-- generated-index:start -->";
pub const INDEX_END: &str = "<!-- generated-index:end -->";

/// The header the artefact carries (invariant 5): a reference page is
/// consulted selectively, from its index, and is not a document to ingest.
pub const REFERENCE_HEADER: &str = "<!-- reference-header -->\n\
     > This page is a reference to consult selectively, not a document to \
     ingest. Start from the index below and read only the sections that apply.";

/// Whether the artefact declares itself a selectively-consulted reference.
pub fn has_reference_header(source: &str) -> bool {
    source.contains(REFERENCE_HEADER)
}

/// Prepend [`REFERENCE_HEADER`] when absent (idempotent).
pub fn ensure_reference_header(source: &str) -> String {
    if has_reference_header(source) {
        return source.to_string();
    }
    if source.is_empty() {
        format!("{REFERENCE_HEADER}\n")
    } else {
        format!("{REFERENCE_HEADER}\n\n{source}")
    }
}

/// Parse ATX headings out of a markdown document, in document order.
///
/// Headings inside fenced code blocks (``` or ~~~) are not headings: a
/// documented example is content, not structure, and counting it would put
/// a link in the index pointing at nothing.
pub fn extract_headings(source: &str) -> Vec<Heading> {
    let mut out = Vec::new();
    let mut fence: Option<char> = None;
    for (idx, raw) in source.lines().enumerate() {
        let line = raw.trim_start();
        match fence {
            Some(mark) => {
                if line.starts_with(&[mark, mark, mark]) {
                    fence = None;
                }
                continue;
            }
            None => {
                if line.starts_with("```") {
                    fence = Some('`');
                    continue;
                }
                if line.starts_with("~~~") {
                    fence = Some('~');
                    continue;
                }
            }
        }
        if let Some((level, title)) = parse_atx(line) {
            out.push(Heading {
                level,
                title,
                line: idx + 1,
            });
        }
    }
    out
}

/// One ATX heading line: 1–6 `#`, then a space or end of line. A seventh
/// `#` is not a heading, and neither is `#hashtag`. Trailing `###` run-off
/// (closed ATX form) is stripped.
fn parse_atx(line: &str) -> Option<(u8, String)> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    let mut title = rest.trim().to_string();
    let trail = title.bytes().rev().take_while(|b| *b == b'#').count();
    if trail > 0 {
        title.truncate(title.len() - trail);
        title = title.trim_end().to_string();
    }
    (!title.is_empty()).then_some((hashes as u8, title))
}

/// GitHub-style anchor for a heading title: lower-cased, punctuation
/// dropped, whitespace and `_` runs collapsed to single `-`.
pub fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.extend(c.to_lowercase());
        } else if c == ' ' || c == '-' || c == '_' || c == '\t' {
            pending_dash = !out.is_empty();
        }
        // Any other character is dropped without becoming a separator, so
        // "Don't" → "dont" and "outages?" → "outages".
    }
    out
}

/// One index entry: a heading plus the anchor it links to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub level: u8,
    pub title: String,
    /// 1-based source line of the heading.
    pub line: usize,
    /// The unique anchor this entry links to (`#slug`).
    pub anchor: String,
    /// The base slug before uniqueness suffixing; equal to `anchor` unless
    /// a collision forced a `-N` suffix.
    pub base_slug: String,
}

impl IndexEntry {
    /// The rendered link text/anchor pair as `title` / `#anchor`.
    pub fn link(&self) -> String {
        format!("[{}]({})", self.title, self.anchor)
    }
}

/// Two or more headings whose titles produce the same base slug. Without
/// suffixing, both index entries would resolve to one section (invariant
/// 4), and the second section would be unaddressable while looking indexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlugCollision {
    pub slug: String,
    /// Every heading line sharing the slug, in document order.
    pub lines: Vec<usize>,
    pub titles: Vec<String>,
}

impl SlugCollision {
    pub fn line(&self) -> String {
        format!(
            "slug collision `#{}`: {} headings at line(s) {} — entries would resolve to one section",
            self.slug,
            self.lines.len(),
            self.lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// The index generated from a document's headings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentIndex {
    /// Entries in document order.
    pub entries: Vec<IndexEntry>,
    /// Headings whose base slug collided (the generated ids are unique; the
    /// collision is reported, not swallowed).
    pub collisions: Vec<SlugCollision>,
    /// Heading count the index was generated from.
    pub headings: usize,
}

impl DocumentIndex {
    /// Invariant 3: generate the index from the document's own structure,
    /// headings to anchored links, in document order.
    ///
    /// A repeated slug is suffixed `-1`, `-2`, … (GitHub's own behaviour) so
    /// the ids stay unique, and the collision is recorded rather than
    /// silently resolved.
    pub fn generate(source: &str) -> Self {
        let headings = extract_headings(source);
        let mut taken: std::collections::BTreeSet<String> = BTreeSet::new();
        let mut entries = Vec::with_capacity(headings.len());
        let mut seen: BTreeMap<String, (Vec<usize>, Vec<String>)> = BTreeMap::new();
        for h in &headings {
            let base = slugify(&h.title);
            let mut anchor = base.clone();
            let mut k = 1usize;
            while taken.contains(&anchor) {
                anchor = format!("{base}-{k}");
                k += 1;
            }
            taken.insert(anchor.clone());
            let e = seen.entry(base.clone()).or_default();
            e.0.push(h.line);
            e.1.push(h.title.clone());
            entries.push(IndexEntry {
                level: h.level,
                title: h.title.clone(),
                line: h.line,
                anchor,
                base_slug: base,
            });
        }
        let mut collisions: Vec<SlugCollision> = seen
            .into_iter()
            .filter(|(_, (lines, _))| lines.len() > 1)
            .map(|(slug, (lines, titles))| SlugCollision {
                slug,
                lines,
                titles,
            })
            .collect();
        collisions.sort_by_key(|c| c.lines.first().copied().unwrap_or(0));
        Self {
            entries,
            collisions,
            headings: headings.len(),
        }
    }

    /// The anchors, in document order.
    pub fn anchors(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.anchor.clone()).collect()
    }

    /// Invariant 3: the rendered index block — one bullet per heading,
    /// indented by level, in document order. Contains no heading lines of
    /// its own, so re-extracting headings from a document that carries the
    /// block does not change the heading count.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            let indent = "  ".repeat(usize::from(e.level.saturating_sub(1)));
            let _ = writeln_out(&mut out, &format!("{}- {} ({})", indent, e.link(), e.line));
        }
        out.trim_end().to_string()
    }

    /// Invariant 4: verify the index by count, not by eye —
    /// heading count == link count == unique id count.
    pub fn verify(&self) -> CountVerdict {
        let links = self.entries.len();
        let ids: BTreeSet<String> = self.entries.iter().map(|e| e.anchor.clone()).collect();
        verify_counts(self.headings, links, ids.len())
    }

    /// Whether the index is empty because the document has no structure to
    /// index.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Write a line without pulling `std::fmt::Write` noise into the caller.
fn writeln_out(out: &mut String, line: &str) -> std::fmt::Result {
    use std::fmt::Write;
    writeln!(out, "{line}")
}

/// Invariant 4: the three counts must be equal.
///
/// `headings` is the structure the index is derived from, `links` is what
/// the index renders, `unique_ids` is how many distinct anchors exist. Two
/// entries pointing at one section (a slug collision that was not
/// suffixed) shows as `links > unique_ids`; an index maintained by hand and
/// left behind shows as `headings != links`.
pub fn verify_counts(headings: usize, links: usize, unique_ids: usize) -> CountVerdict {
    if headings == links && links == unique_ids {
        CountVerdict::Consistent { count: headings }
    } else {
        CountVerdict::Inconsistent(CountGap {
            headings,
            links,
            unique_ids,
        })
    }
}

/// The concrete counts behind a failed verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountGap {
    pub headings: usize,
    pub links: usize,
    pub unique_ids: usize,
}

impl CountGap {
    pub fn line(&self) -> String {
        let mut why = Vec::new();
        if self.headings != self.links {
            why.push(format!(
                "{} heading(s) but {} link(s): the index is not generated from the document \
                 (hand-maintained, or a section was added without it)",
                self.headings, self.links
            ));
        }
        if self.links != self.unique_ids {
            why.push(format!(
                "{} link(s) but {} unique id(s): a slug collision points {} entr(ies) at one section",
                self.links,
                self.unique_ids,
                self.links - self.unique_ids
            ));
        }
        format!(
            "index counts disagree (headings={} links={} unique_ids={}): {}",
            self.headings,
            self.links,
            self.unique_ids,
            why.join("; ")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountVerdict {
    Consistent { count: usize },
    Inconsistent(CountGap),
}

impl CountVerdict {
    pub fn is_consistent(&self) -> bool {
        matches!(self, CountVerdict::Consistent { .. })
    }

    pub fn line(&self) -> String {
        match self {
            CountVerdict::Consistent { count } => {
                format!("index counts consistent: {count} headings = {count} links = {count} ids")
            }
            CountVerdict::Inconsistent(gap) => gap.line(),
        }
    }
}

/// Invariant 1: the index is required from the first entry.
///
/// There is no threshold at which an index becomes worthwhile — that
/// threshold is the artefact's own growth history, and every accumulating
/// document has already passed it by the time the need is visible.
pub fn index_required(entry_count: usize) -> bool {
    entry_count >= 1
}

/// The state of the index block carried by a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexStatus {
    /// The document has no headings: there is nothing to index yet.
    NothingToIndex,
    /// The document has headings but no generated index block — invariant 1
    /// violated from the first entry.
    Missing { headings: usize },
    /// The block matches the structure of the document.
    Current { entries: usize },
    /// The block disagrees with the document's structure: it was typed, or
    /// written against an older revision of the document.
    Drifted(IndexDrift),
}

impl IndexStatus {
    pub fn is_finding(&self) -> bool {
        matches!(self, IndexStatus::Missing { .. } | IndexStatus::Drifted(_))
    }

    pub fn line(&self) -> String {
        match self {
            IndexStatus::NothingToIndex => "index: nothing to index (no headings)".to_string(),
            IndexStatus::Missing { headings } => format!(
                "index: MISSING — {headings} heading(s) and no generated index block \
                 (an index is required from the first entry)"
            ),
            IndexStatus::Current { entries } => {
                format!("index: current ({entries} entries generated from structure)")
            }
            IndexStatus::Drifted(drift) => drift.line(),
        }
    }
}

/// How an existing index block disagrees with the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexDrift {
    /// Present in the document's structure, absent from the block.
    pub missing: Vec<String>,
    /// Present in the block, absent from the document's structure (a stale
    /// entry for a renamed or deleted section).
    pub stale: Vec<String>,
    /// Headings and links exist in equal number but disagree on anchor or
    /// title.
    pub headings: usize,
    pub links: usize,
}

impl IndexDrift {
    pub fn line(&self) -> String {
        let mut parts = vec![format!(
            "index: DRIFTED — the block does not match the document's structure \
             (headings={} links={})",
            self.headings, self.links
        )];
        if !self.missing.is_empty() {
            parts.push(format!("missing {} entry(ies): {}", self.missing.len(), self.missing.join(", ")));
        }
        if !self.stale.is_empty() {
            parts.push(format!("stale {} entry(ies): {}", self.stale.len(), self.stale.join(", ")));
        }
        parts.join("; ")
    }
}

/// The index block currently embedded in `source`, if any.
pub fn index_block(source: &str) -> Option<String> {
    let start = source.find(INDEX_BEGIN)?;
    let rest = &source[start + INDEX_BEGIN.len()..];
    let end = rest.find(INDEX_END)?;
    Some(rest[..end].trim().to_string())
}

/// Parse the `(title, anchor)` pairs out of a rendered index block.
fn parse_index_links(block: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in block.lines() {
        let t = line.trim_start();
        let Some(bullet) = t.strip_prefix("- ") else {
            continue;
        };
        let Some(open) = bullet.find('[') else { continue };
        let after_open = &bullet[open + 1..];
        let Some(close) = after_open.find("](") else { continue };
        let title = after_open[..close].to_string();
        let after_link = &after_open[close + 2..];
        let Some(paren) = after_link.find(')') else { continue };
        let anchor = after_link[..paren].to_string();
        out.push((title, anchor));
    }
    out
}

/// Invariant 3: an index beside the document is only trustworthy if it is
/// the index the document's structure generates. Compare the embedded block
/// against a freshly generated one and report the difference.
pub fn index_status(source: &str) -> IndexStatus {
    let generated = DocumentIndex::generate(source);
    index_status_for(source, &generated)
}

/// [`index_status`] with an already-generated index (same comparison).
pub fn index_status_for(source: &str, generated: &DocumentIndex) -> IndexStatus {
    if generated.is_empty() {
        return IndexStatus::NothingToIndex;
    }
    let Some(block) = index_block(source) else {
        return IndexStatus::Missing {
            headings: generated.headings,
        };
    };
    let links = parse_index_links(&block);
    let want: Vec<(String, String)> = generated
        .entries
        .iter()
        .map(|e| (e.title.clone(), e.anchor.clone()))
        .collect();
    if links == want {
        return IndexStatus::Current {
            entries: generated.entries.len(),
        };
    }
    let have: BTreeSet<(String, String)> = links.iter().cloned().collect();
    let want_set: BTreeSet<(String, String)> = want.iter().cloned().collect();
    let missing = want_set
        .difference(&have)
        .map(|(t, a)| format!("{t} ({a})"))
        .collect();
    let stale = have
        .difference(&want_set)
        .map(|(t, a)| format!("{t} ({a})"))
        .collect();
    IndexStatus::Drifted(IndexDrift {
        missing,
        stale,
        headings: generated.headings,
        links: links.len(),
    })
}

use std::collections::BTreeSet;
use std::fmt::Write as _unused_write;

/// Replace (or insert) the generated index block in `source` with the index
/// generated from its own headings. Idempotent: running it twice on the
/// same document changes nothing.
///
/// The block is inserted after the document's first heading (or at the top
/// when there is none) on first write, and replaced in place afterwards.
pub fn regenerate_index(source: &str) -> String {
    let generated = DocumentIndex::generate(source);
    let block = format!("{INDEX_BEGIN}\n{}\n{INDEX_END}", generated.render());
    match (source.find(INDEX_BEGIN), source.find(INDEX_END)) {
        (Some(start), Some(end)) if end > start => {
            let tail = &source[end + INDEX_END.len()..];
            format!("{}{}{}", &source[..start], block, tail)
        }
        _ => insert_block(source, &block),
    }
}

fn insert_block(source: &str, block: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    // Insert after the first heading line, keeping the block below the
    // document's own title. Heading lines are not inside the block here:
    // the caller has not inserted it yet.
    let at = extract_headings(source)
        .first()
        .map(|h| h.line)
        .unwrap_or(0);
    let mut out = String::new();
    if at == 0 {
        let _ = writeln_out(&mut out, "{block}");
        if !source.is_empty() {
            out.push('\n');
        }
        out.push_str(source);
        return out;
    }
    for (idx, line) in lines.iter().enumerate() {
        let _ = writeln_out(&mut out, "{line}");
        if idx + 1 == at {
            out.push('\n');
            let _ = writeln_out(&mut out, "{block}");
        }
    }
    out
}

// ── Invariant 2 applied: the write gate ─────────────────────────────────────

/// Where a document may be split: the heading level whose sections are the
/// split boundaries. A page of 328 `##` sections splits at `##`.
pub const DEFAULT_SPLIT_LEVEL: u8 = 2;

/// A mechanical split plan: the line numbers of the section headings that
/// start a new part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPlan {
    /// Number of parts the document would be cut into.
    pub parts: usize,
    /// 1-based heading lines where a new part begins (the first part starts
    /// at the top of the document and is not listed).
    pub break_at: Vec<usize>,
    pub tokens: usize,
    pub max: usize,
    /// A single section already exceeds the budget, so no heading boundary
    /// can produce a fitting part. The document needs a human trim, not a
    /// mechanical split.
    pub unsplitable_section: Option<usize>,
}

impl SplitPlan {
    pub fn line(&self) -> String {
        match self.unsplitable_section {
            Some(line) => format!(
                "size: ~{} tokens (budget {}) — OVER, cannot split: the section at line {line} \
                 alone exceeds the budget; trim it by hand",
                self.tokens, self.max
            ),
            None => format!(
                "size: ~{} tokens (budget {}) — OVER by {}, split into {} part(s) at heading line(s) {}",
                self.tokens,
                self.max,
                self.tokens - self.max,
                self.parts,
                if self.break_at.is_empty() {
                    "—".to_string()
                } else {
                    self.break_at
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ),
        }
    }
}

/// The decision taken at one write. Every arm renders the token estimate:
/// the estimate is emitted on every write (invariant 2), not only when the
/// write is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteAction {
    /// Under budget: write it.
    Write { tokens: usize, max: usize },
    /// Over budget and mechanically splittable at section headings.
    Split(SplitPlan),
    /// Over budget and not mechanically splittable — refuse the write.
    Refuse {
        tokens: usize,
        max: usize,
        reason: String,
    },
}

impl WriteAction {
    pub fn is_write(&self) -> bool {
        matches!(self, WriteAction::Write { .. })
    }

    pub fn tokens(&self) -> usize {
        match self {
            WriteAction::Write { tokens, .. } => *tokens,
            WriteAction::Split(plan) => plan.tokens,
            WriteAction::Refuse { tokens, .. } => *tokens,
        }
    }

    /// The line emitted on the write, whatever the outcome.
    pub fn line(&self) -> String {
        match self {
            WriteAction::Write { tokens, max } => {
                format!("size: ~{tokens} tokens (budget {max}) — within, write")
            }
            WriteAction::Split(plan) => plan.line(),
            WriteAction::Refuse {
                tokens,
                max,
                reason,
            } => format!("size: ~{tokens} tokens (budget {max}) — REFUSED: {reason}"),
        }
    }
}

/// Invariant 2: refuse or split past the threshold.
///
/// Within budget → [`WriteAction::Write`]. Over budget → a greedy split at
/// `split_level` headings, each part within the budget. A document whose
/// single longest section already exceeds the budget cannot be split
/// mechanically, and the write is refused rather than published oversized:
/// a split that produces a part over budget is the same artefact with extra
/// steps.
pub fn write_action(source: &str, budget: &SizeBudget) -> WriteAction {
    match budget.verdict(source) {
        BudgetVerdict::Within { tokens, max, .. } => WriteAction::Write { tokens, max },
        BudgetVerdict::Exceeded {
            tokens,
            max,
            over_by,
            ..
        } => {
            let plan = split_plan(source, budget, DEFAULT_SPLIT_LEVEL);
            match plan.unsplitable_section {
                Some(line) => WriteAction::Refuse {
                    tokens,
                    max,
                    reason: format!(
                        "the section starting at line {line} is itself over budget by \
                         (over by {over_by} in total); trim it — no heading split fits"
                    ),
                },
                None if plan.parts <= 1 => WriteAction::Refuse {
                    tokens,
                    max,
                    reason: format!(
                        "no level-{DEFAULT_SPLIT_LEVEL} heading boundary exists to split at \
                         (over by {over_by}); add structure or trim"
                    ),
                },
                None => WriteAction::Split(plan),
            }
        }
    }
}

/// Greedy section packing: walk the sections delimited by headings at
/// `split_level` (plus any preamble before the first one), accumulate parts
/// while they fit, break before the section that would overflow.
pub fn split_plan(source: &str, budget: &SizeBudget, split_level: u8) -> SplitPlan {
    let lines: Vec<&str> = source.lines().collect();
    let starts: Vec<usize> = extract_headings(source)
        .into_iter()
        .filter(|h| h.level == split_level)
        .map(|h| h.line - 1)
        .collect();
    let mut sections: Vec<(usize, usize)> = Vec::new();
    if starts.is_empty() {
        sections.push((0, lines.len()));
    } else {
        if starts[0] > 0 {
            sections.push((0, starts[0]));
        }
        for (i, s) in starts.iter().enumerate() {
            let end = starts.get(i + 1).copied().unwrap_or(lines.len());
            sections.push((*s, end));
        }
    }

    let mut parts = 0usize;
    let mut break_at = Vec::new();
    let mut current_tokens = 0usize;
    let mut unsplitable_section = None;
    for (start, end) in &sections {
        let body = lines[*start..*end].join("\n");
        let t = budget.estimate(&body);
        if t > budget.max_tokens && unsplitable_section.is_none() {
            // A part can never fit this section, whatever breaks surround it.
            unsplitable_section = Some(start + 1);
        }
        if current_tokens == 0 {
            current_tokens = t;
        } else if current_tokens + t <= budget.max_tokens {
            current_tokens += t;
        } else {
            parts += 1;
            break_at.push(start + 1);
            current_tokens = t;
        }
    }
    parts += 1;
    if !break_at.is_empty() {
        // The final part closes the document; only interior breaks are listed
        // plus the last one, which is already pushed above.
    }

    SplitPlan {
        parts,
        break_at,
        tokens: budget.estimate(source),
        max: budget.max_tokens,
        unsplitable_section,
    }
}

// ── The composed write path ─────────────────────────────────────────────────

/// What one write of an accumulating artefact produces: the reference
/// header, the regenerated index block, and the budget decision measured on
/// the prepared content (the index itself costs tokens, and a budget that
/// excludes it lies about the size of what is written).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePlan {
    /// Header + document + regenerated index, ready to publish.
    pub content: String,
    /// The budget decision on `content`.
    pub action: WriteAction,
    /// The index generated from the document's structure.
    pub index: DocumentIndex,
    /// The index-status verdict for the *input* document (whether the
    /// caller's own index block was missing or drifted).
    pub status: IndexStatus,
    /// Invariant 4's count verification over the generated index.
    pub counts: CountVerdict,
}

impl WritePlan {
    /// The one-line report a write emits.
    pub fn line(&self) -> String {
        format!(
            "artefact: headings={} ids={} {}; {}",
            self.index.headings,
            self.index.anchors().len(),
            self.counts.line(),
            self.action.line()
        )
    }

    /// Whether this write may be published as-is.
    pub fn is_publishable(&self) -> bool {
        self.action.is_write() && self.counts.is_consistent()
    }
}

/// The write path for an accumulating artefact: ensure the reference header
/// (invariant 5), regenerate the index from structure (invariants 1 and 3),
/// verify by count (invariant 4), gate on size (invariant 2).
pub fn prepare_write(source: &str, budget: &SizeBudget) -> WritePlan {
    let status = index_status(source);
    let with_header = ensure_reference_header(source);
    let content = regenerate_index(&with_header);
    let index = DocumentIndex::generate(&content);
    let counts = index.verify();
    let action = write_action(&content, budget);
    WritePlan {
        content,
        action,
        index,
        status,
        counts,
    }
}

/// A report over one artefact: every invariant on one line, findings named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditReport {
    pub headings: usize,
    pub entries: usize,
    pub unique_ids: usize,
    pub tokens: usize,
    pub max: usize,
    pub findings: Vec<String>,
}

impl AuditReport {
    pub fn ok(&self) -> bool {
        self.findings.is_empty()
    }

    /// The line a periodic audit emits: the counts first (invariant 4's
    /// denominator), then every finding by name.
    pub fn line(&self) -> String {
        let head = format!(
            "headings={} index={} ids={} tokens~{}/{} findings={}",
            self.headings,
            self.entries,
            self.unique_ids,
            self.tokens,
            self.max,
            self.findings.len()
        );
        if self.findings.is_empty() {
            head
        } else {
            format!("{head}: {}", self.findings.join(" | "))
        }
    }
}

/// Run every invariant over `source` and report.
pub fn audit(source: &str, budget: &SizeBudget) -> AuditReport {
    let plan = prepare_write(source, budget);
    let mut findings = Vec::new();
    if plan.status.is_finding() {
        findings.push(plan.status.line());
    }
    if !plan.counts.is_consistent() {
        findings.push(plan.counts.line());
    }
    if !plan.action.is_write() {
        findings.push(plan.action.line());
    }
    for c in &plan.index.collisions {
        findings.push(c.line());
    }
    if !has_reference_header(source) {
        findings.push(
            "reference header missing: the page does not state that it is a reference to consult \
             selectively, not a document to ingest"
                .to_string(),
        );
    }
    AuditReport {
        headings: plan.index.headings,
        entries: plan.index.entries.len(),
        unique_ids: plan.index.anchors().len(),
        tokens: plan.action.tokens(),
        max: budget.max_tokens,
        findings,
    }
}
