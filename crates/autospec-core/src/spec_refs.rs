//! Dispatch-time resolution of `specs/NN` references (issue #3750).
//!
//! A task card may cite canonical specs (`specs/03`, `specs/26`) held in a
//! separate repository. On the HPC cluster that repository is not
//! fetchable — no authenticated `gh`, limited outbound access — so an agent
//! that resolves the reference at implementation time implements from the
//! card's lossy prose summary and records the limitation in prose. Nothing
//! in the pipeline notices that an authoritative reference went unread, and
//! the reviewer reads the same summary.
//!
//! This module moves resolution to *dispatch* time, where the dispatching
//! side has authenticated access:
//!
//! 1. [`extract_spec_references`] finds every reference the card cites.
//! 2. [`SpecResolution::resolve`] fetches each one through
//!    [`SpecFetcher`] and splits the outcome into inlined and unresolved;
//!    [`SpecResolution::inline_into`] appends the fetched text to the
//!    context pack, so the agent reads the spec rather than a summary of
//!    it.
//! 3. [`SpecResolution::status_lines`] renders the block recorded in
//!    `status.txt`: which references were requested, which were inlined,
//!    and which could not be — a measurable property of the run, not a
//!    footnote in prose.
//! 4. [`resolve_dispatch_context`] sizes the context class
//!    ([`class_for_tokens`]) on the *post-resolution* pack, so inlining
//!    specs cannot silently overflow the class the dispatch was planned
//!    for.
//!
//! Completion reports that carry an unresolved-reference caveat are
//! flagged for review instead of being treated as ordinary successes
//! ([`classify_completion`]).
//!
//! The module performs no I/O: the caller fetches spec text (e.g. through
//! authenticated `gh`) and persists the returned artifacts, mirroring the
//! workspace convention that the core is pure.

use serde::{Deserialize, Serialize};

use crate::rag::compression::estimate_tokens;

/// Extract the canonical spec references (`specs/NN`) cited in a card.
///
/// A reference is `specs/` followed by one or more digits, optionally
/// followed by a `-slug`. It must not be embedded in a longer path or
/// identifier: the character before `specs/` may not be alphanumeric, `-`
/// or `/`, so `docs/specs/03`, `myspecs/03` and `x-specs/03` are not
/// references, and a character immediately after the number (or slug) must
/// not be alphanumeric, so `specs/03x` is not a reference.
///
/// References are deduplicated and returned in first-mention order.
pub fn extract_spec_references(card_text: &str) -> Vec<String> {
    let chars: Vec<char> = card_text.chars().collect();
    let mut references = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut i = 0;
    while i + 6 <= chars.len() {
        let start: String = chars[i..i + 6].iter().collect();
        if start != "specs/" {
            i += 1;
            continue;
        }
        let preceded_by_path = {
            let previous = chars.get(i - 1).copied();
            previous
                .map(|c| c.is_ascii_alphanumeric() || c == '-' || c == '/')
                .unwrap_or(false)
        };
        if preceded_by_path {
            i += 1;
            continue;
        }
        match scan_reference_end(&chars, i + 6) {
            Some(end) => {
                let token: String = chars[i..end].iter().collect();
                if seen.insert(token.clone()) {
                    references.push(token);
                }
                i = end;
            }
            None => i += 1,
        }
    }
    references
}

/// End index of a reference that starts its digit run at `from`, or `None`
/// when no digits (or an alphanumeric continuation) follows.
fn scan_reference_end(chars: &[char], from: usize) -> Option<usize> {
    let n = chars.len();
    let mut digits = from;
    while digits < n && chars[digits].is_ascii_digit() {
        digits += 1;
    }
    if digits == from {
        return None;
    }
    let mut end = digits;
    if end < n && chars[end] == '-' {
        let mut k = end + 1;
        while k < n && (chars[k].is_ascii_alphanumeric() || chars[k] == '-') {
            k += 1;
        }
        while k > end + 1 && chars[k - 1] == '-' {
            k -= 1;
        }
        if k > end + 1 {
            end = k;
        }
    }
    if end < n && chars[end].is_ascii_alphanumeric() {
        return None;
    }
    Some(end)
}

/// A spec the dispatching side can fetch (e.g. through authenticated `gh`).
///
/// The core performs no I/O; the caller implements the fetch and supplies
/// the clock and credentials.
pub trait SpecFetcher {
    /// Return the canonical text of one spec reference, or an error
    /// recorded against the reference.
    fn fetch(&self, reference: &str) -> Result<String, String>;
}

/// One reference whose canonical text was fetched and inlined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlinedSpec {
    /// The reference exactly as cited in the card.
    pub reference: String,
    /// The canonical text, verbatim.
    pub text: String,
    /// The estimated token cost of [`Self::text`].
    pub tokens: u32,
}

/// One reference the dispatch could not inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedSpec {
    /// The reference exactly as cited in the card.
    pub reference: String,
    /// Why the dispatch could not inline it (fetch error or empty body).
    pub reason: String,
}

/// The outcome of resolving a card's cited references at dispatch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecResolution {
    /// Every reference the card cited, first-mention order.
    pub requested: Vec<String>,
    /// The references whose text was fetched and inlined, request order.
    pub inlined: Vec<InlinedSpec>,
    /// The references that could not be fetched, request order.
    pub unresolved: Vec<UnresolvedSpec>,
}

impl SpecResolution {
    /// Fetch every requested reference through `fetch` and split the
    /// outcome into inlined and unresolved, preserving request order.
    ///
    /// An error, and an empty body (a fetch that returned nothing
    /// readable), both leave the reference unresolved.
    pub fn resolve(requested: Vec<String>, fetch: &dyn SpecFetcher) -> Self {
        let mut inlined = Vec::new();
        let mut unresolved = Vec::new();
        for reference in &requested {
            match fetch.fetch(reference) {
                Ok(text) if !text.trim().is_empty() => inlined.push(InlinedSpec {
                    reference: reference.clone(),
                    tokens: estimate_tokens(&text),
                    text,
                }),
                Ok(_) => unresolved.push(UnresolvedSpec {
                    reference: reference.clone(),
                    reason: "fetch returned an empty body".to_string(),
                }),
                Err(reason) => unresolved.push(UnresolvedSpec {
                    reference: reference.clone(),
                    reason,
                }),
            }
        }
        Self {
            requested,
            inlined,
            unresolved,
        }
    }

    /// Append every inlined spec to the context pack so the implementing
    /// agent reads the canonical text rather than the card's summary.
    ///
    /// The pack is returned unchanged when nothing was inlined.
    pub fn inline_into(&self, context_pack: &str) -> String {
        if self.inlined.is_empty() {
            return context_pack.to_string();
        }
        let mut out = context_pack.to_string();
        for spec in &self.inlined {
            out.push_str("\n\n## Canonical spec `");
            out.push_str(&spec.reference);
            out.push_str("`, fetched at dispatch\n\n");
            out.push_str(&spec.text);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }

    /// The estimated token cost of all inlined specs.
    pub fn inlined_tokens(&self) -> u32 {
        self.inlined.iter().map(|spec| spec.tokens).sum()
    }

    /// True when every requested reference was inlined.
    pub fn is_complete(&self) -> bool {
        self.unresolved.is_empty()
    }

    /// The lines recorded in `status.txt`: which references were
    /// requested, which were inlined, and which could not be — a
    /// measurable property of the run, not a footnote in prose.
    pub fn status_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "spec-refs requested={} inlined={} unresolved={}",
            self.requested.len(),
            self.inlined.len(),
            self.unresolved.len()
        )];
        for spec in &self.inlined {
            lines.push(format!(
                "spec-ref {} inlined tokens={}",
                spec.reference, spec.tokens
            ));
        }
        for unresolved in &self.unresolved {
            let reason: String =
                unresolved.reason.split_whitespace().collect::<Vec<_>>().join(" ");
            lines.push(format!(
                "spec-ref {} unresolved reason={}",
                unresolved.reference, reason
            ));
        }
        lines
    }
}

/// A dispatch context class: the share of the model window the context
/// pack may occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ContextClass {
    /// The pack may occupy at most 65 percent of the window.
    C65,
    /// The pack may occupy at most 90 percent of the window.
    C90,
}

impl ContextClass {
    /// The share of the window (percent) the class permits.
    pub fn occupancy_percent(self) -> u32 {
        match self {
            ContextClass::C65 => 65,
            ContextClass::C90 => 90,
        }
    }

    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            ContextClass::C65 => "C65",
            ContextClass::C90 => "C90",
        }
    }

    /// Parse the stable wire name.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "C65" => Ok(ContextClass::C65),
            "C90" => Ok(ContextClass::C90),
            other => Err(format!("unknown context class: {other}")),
        }
    }
}

/// The largest pack (tokens) a class permits on a window this large.
fn class_ceiling(class: ContextClass, window_tokens: u32) -> u32 {
    window_tokens.saturating_mul(class.occupancy_percent()) / 100
}

/// Size the context class for a pack: the smallest class whose ceiling
/// fits the pack, or `None` when the pack overflows even the largest
/// class.
///
/// Must be called on the *post-resolution* pack — the text with inlined
/// specs — or the added tokens silently overflow the chosen class.
pub fn class_for_tokens(pack_tokens: u32, window_tokens: u32) -> Option<ContextClass> {
    if pack_tokens <= class_ceiling(ContextClass::C65, window_tokens) {
        Some(ContextClass::C65)
    } else if pack_tokens <= class_ceiling(ContextClass::C90, window_tokens) {
        Some(ContextClass::C90)
    } else {
        None
    }
}

/// The dispatch context after its spec references have been resolved: the
/// resolution record, the pack with specs inlined, the `status.txt` block,
/// and the context class sized on the post-resolution pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedDispatchContext {
    /// The resolution record (requested / inlined / unresolved).
    pub resolution: SpecResolution,
    /// The context pack with every inlined spec appended.
    pub pack: String,
    /// The block recorded in `status.txt`.
    pub status: Vec<String>,
    /// The estimated token cost of [`Self::pack`].
    pub pack_tokens: u32,
    /// The context class sized on the post-resolution pack, or `None`
    /// when the pack overflows every class.
    pub class: Option<ContextClass>,
}

/// Resolve every `specs/NN` reference the card cites, inline the fetched
/// spec text into the context pack, record the outcome, and size the
/// context class *after* inlining so the added tokens cannot silently
/// overflow the class the dispatch was planned for.
pub fn resolve_dispatch_context(
    card_text: &str,
    window_tokens: u32,
    fetch: &dyn SpecFetcher,
) -> ResolvedDispatchContext {
    let requested = extract_spec_references(card_text);
    let resolution = SpecResolution::resolve(requested, fetch);
    let pack = resolution.inline_into(card_text);
    let pack_tokens = estimate_tokens(&pack);
    let class = class_for_tokens(pack_tokens, window_tokens);
    let status = resolution.status_lines();
    ResolvedDispatchContext {
        resolution,
        pack,
        status,
        pack_tokens,
        class,
    }
}

/// Phrases that admit a cited spec could not be read.
const CAVEAT_PHRASES: &[&str] = &[
    "not reachable",
    "not fetchable",
    "not fetched",
    "not available",
    "could not fetch",
    "unable to fetch",
    "cannot fetch",
    "can't fetch",
    "no access",
    "unreachable",
];

/// True when a completion report carries an unresolved-reference caveat:
/// a sentence that both admits a spec could not be read and names an
/// unresolved reference or a spec.
///
/// Deliberately conservative — a false flag costs a review, while a missed
/// caveat is the invisible divergence this module exists to catch.
pub fn has_unresolved_reference_caveat(report: &str, unresolved: &[String]) -> bool {
    let lowered = report.to_lowercase();
    lowered.split(['.', '!', '?', '\n']).any(|sentence| {
        if !CAVEAT_PHRASES
            .iter()
            .any(|phrase| sentence.contains(phrase))
        {
            return false;
        }
        unresolved
            .iter()
            .any(|reference| sentence.contains(reference.as_str()))
            || sentence.contains("spec")
    })
}

/// How a completion report should be treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompletionOutcome {
    /// An ordinary success: the report carries no unresolved-reference
    /// caveat.
    Success,
    /// The report admits an authoritative reference went unread; review it
    /// instead of accepting it as an ordinary success.
    FlaggedForReview,
}

impl CompletionOutcome {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            CompletionOutcome::Success => "SUCCESS",
            CompletionOutcome::FlaggedForReview => "FLAGGED_FOR_REVIEW",
        }
    }
}

/// Classify a completion report against the dispatch's resolution record.
///
/// A report carrying an unresolved-reference caveat is flagged for review
/// instead of being treated as an ordinary success. A report that carries
/// no caveat while references remain unresolved is not a report property —
/// the gap is already a measurable property of the run in `status.txt`.
pub fn classify_completion(report: &str, resolution: &SpecResolution) -> CompletionOutcome {
    if !resolution.unresolved.is_empty()
        && has_unresolved_reference_caveat(
            report,
            &resolution.unresolved.iter().map(|u| u.reference.clone()).collect::<Vec<_>>(),
        )
    {
        CompletionOutcome::FlaggedForReview
    } else {
        CompletionOutcome::Success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct MapFetcher(BTreeMap<String, Result<String, String>>);

    impl SpecFetcher for MapFetcher {
        fn fetch(&self, reference: &str) -> Result<String, String> {
            self.0
                .get(reference)
                .cloned()
                .unwrap_or_else(|| Err(format!("no access to {reference} from this environment")))
        }
    }

    #[test]
    fn extract_finds_references_in_first_mention_order_and_dedupes() {
        let card = "Build the sampler per `specs/03`. The buffer rules are in specs/26. \
                    See specs/03 again for the API.";
        assert_eq!(
            extract_spec_references(card),
            vec!["specs/03".to_string(), "specs/26".to_string()]
        );
    }

    #[test]
    fn extract_rejects_path_and_identifier_embeddings() {
        assert_eq!(
            extract_spec_references("read docs/specs/03 in the repo, myspecs/04, x-specs/05"),
            Vec::<String>::new()
        );
        assert_eq!(extract_spec_references("see specs/03x for details"), Vec::<String>::new());
    }

    #[test]
    fn extract_accepts_punctuation_boundaries_and_slugs() {
        assert_eq!(
            extract_spec_references("(specs/03; specs/26,specs/27)"),
            vec![
                "specs/03".to_string(),
                "specs/26".to_string(),
                "specs/27".to_string(),
            ]
        );
        assert_eq!(
            extract_spec_references("the quant plan is specs/12-quant and specs/13-"),
            vec!["specs/12-quant".to_string(), "specs/13".to_string()]
        );
    }

    #[test]
    fn resolve_splits_inlined_and_unresolved_preserving_order() {
        let fetch = MapFetcher(BTreeMap::from([(
            "specs/03".to_string(),
            Ok("aaaa bbbb cccc dddd".to_string()),
        )]));
        let resolution = SpecResolution::resolve(
            vec![
                "specs/03".to_string(),
                "specs/26".to_string(),
            ],
            &fetch,
        );
        assert_eq!(resolution.requested, vec!["specs/03", "specs/26"]);
        assert_eq!(resolution.inlined, vec![InlinedSpec {
            reference: "specs/03".to_string(),
            text: "aaaa bbbb cccc dddd".to_string(),
            tokens: 5,
        }]);
        assert_eq!(
            resolution.unresolved,
            vec![UnresolvedSpec {
                reference: "specs/26".to_string(),
                reason: "no access to specs/26 from this environment".to_string(),
            }]
        );
        assert!(!resolution.is_complete());
    }

    #[test]
    fn resolve_treats_empty_body_as_unresolved() {
        let fetch = MapFetcher(BTreeMap::from([
            ("specs/03".to_string(), Ok("   \n ".to_string())),
            ("specs/26".to_string(), Err("rate limited".to_string())),
        ]));
        let resolution = SpecResolution::resolve(
            vec!["specs/03".to_string(), "specs/26".to_string()],
            &fetch,
        );
        assert!(resolution.inlined.is_empty());
        assert_eq!(
            resolution.unresolved,
            vec![
                UnresolvedSpec {
                    reference: "specs/03".to_string(),
                    reason: "fetch returned an empty body".to_string(),
                },
                UnresolvedSpec {
                    reference: "specs/26".to_string(),
                    reason: "rate limited".to_string(),
                },
            ]
        );
        assert_eq!(
            resolution.status_lines(),
            vec![
                "spec-refs requested=2 inlined=0 unresolved=2",
                "spec-ref specs/03 unresolved reason=fetch returned an empty body",
                "spec-ref specs/26 unresolved reason=rate limited",
            ]
        );
    }

    #[test]
    fn inline_into_appends_canonical_sections_in_request_order() {
        let fetch = MapFetcher(BTreeMap::from([
            ("specs/03".to_string(), Ok("Spec three body.".to_string())),
            ("specs/26".to_string(), Ok("Spec twenty-six body.".to_string())),
        ]));
        let resolution = SpecResolution::resolve(
            vec!["specs/03".to_string(), "specs/26".to_string()],
            &fetch,
        );
        let pack = resolution.inline_into("Task card prose.");
        assert_eq!(
            pack,
            "Task card prose.\n\n## Canonical spec `specs/03`, fetched at dispatch\n\nSpec \
             three body.\n\n## Canonical spec `specs/26`, fetched at dispatch\n\nSpec \
             twenty-six body.\n"
        );
    }

    #[test]
    fn inline_into_leaves_pack_unchanged_when_nothing_inlined() {
        let resolution = SpecResolution::default();
        assert_eq!(resolution.inline_into("Task card prose."), "Task card prose.");
    }

    #[test]
    fn status_lines_record_requested_inlined_and_unresolved() {
        let fetch = MapFetcher(BTreeMap::from([(
            "specs/03".to_string(),
            Ok("aaaa bbbb cccc dddd".to_string()),
        )]));
        let resolution = SpecResolution::resolve(
            vec!["specs/03".to_string(), "specs/26".to_string()],
            &fetch,
        );
        assert_eq!(
            resolution.status_lines(),
            vec![
                "spec-refs requested=2 inlined=1 unresolved=1",
                "spec-ref specs/03 inlined tokens=5",
                "spec-ref specs/26 unresolved reason=no access to specs/26 from this \
                 environment",
            ]
        );
    }

    #[test]
    fn class_boundaries() {
        assert_eq!(class_for_tokens(0, 1000), Some(ContextClass::C65));
        assert_eq!(class_for_tokens(650, 1000), Some(ContextClass::C65));
        assert_eq!(class_for_tokens(651, 1000), Some(ContextClass::C90));
        assert_eq!(class_for_tokens(900, 1000), Some(ContextClass::C90));
        assert_eq!(class_for_tokens(901, 1000), None);
        assert_eq!(ContextClass::parse("C90").unwrap(), ContextClass::C90);
        assert!(ContextClass::parse("C120").is_err());
    }

    fn repeated_word(tokens_approx: u32) -> String {
        // "word" x N plus separating spaces: tokens = max(ceil(chars/4), N).
        let n = tokens_approx;
        let text = (0..n).map(|_| "word").collect::<Vec<_>>().join(" ");
        assert!(estimate_tokens(&text) >= tokens_approx.saturating_sub(50));
        text
    }

    #[test]
    fn resolve_dispatch_context_sizes_class_after_inlining() {
        let card = "Build the sampler per specs/03.";
        let small = repeated_word(300);
        let small_fetch = MapFetcher(BTreeMap::from([(
            "specs/03".to_string(),
            Ok(small.clone()),
        )]));
        let context = resolve_dispatch_context(card, 1000, &small_fetch);
        assert_eq!(context.class, Some(ContextClass::C90));
        assert!(context.pack.contains("## Canonical spec `specs/03`, fetched at dispatch"));
        assert!(context.pack.contains(&small));
        assert!(!context.resolution.is_complete() == false);
        assert_eq!(
            context.status,
            vec![
                "spec-refs requested=1 inlined=1 unresolved=0",
                format!("spec-ref specs/03 inlined tokens={}", estimate_tokens(&small)),
            ]
        );
    }

    #[test]
    fn resolve_dispatch_context_reports_overflow_when_specs_exceed_every_class() {
        let card = "Build the sampler per specs/03.";
        let huge = repeated_word(1500);
        let fetch = MapFetcher(BTreeMap::from([(
            "specs/03".to_string(),
            Ok(huge),
        )]));
        let context = resolve_dispatch_context(card, 1000, &fetch);
        assert_eq!(context.class, None);
        assert!(context.pack_tokens > class_ceiling(ContextClass::C90, 1000));
    }

    #[test]
    fn caveat_is_detected_when_report_admits_unresolved_reference() {
        let report = "`specs/03`/`specs/26` were not reachable from this environment, so the \
                      implementation follows the issue text. I'm recording that limitation.";
        assert!(has_unresolved_reference_caveat(
            report,
            &["specs/03".to_string(), "specs/26".to_string()]
        ));
        assert!(!has_unresolved_reference_caveat(
            "Implemented per the inlined specs/03. All tests pass.",
            &["specs/26".to_string()]
        ));
        assert!(has_unresolved_reference_caveat(
            "the spec was not fetched from the registry.",
            &["specs/03".to_string()]
        ));
    }

    #[test]
    fn classify_completion_flags_caveats_only_when_references_are_unresolved() {
        let unresolved_resolution = SpecResolution {
            requested: vec!["specs/26".to_string()],
            inlined: Vec::new(),
            unresolved: vec![UnresolvedSpec {
                reference: "specs/26".to_string(),
                reason: "no access".to_string(),
            }],
        };
        let caveated = "specs/26 was not reachable from this environment.";
        assert_eq!(
            classify_completion(caveated, &unresolved_resolution),
            CompletionOutcome::FlaggedForReview
        );
        assert_eq!(
            classify_completion("Done per specs/26.", &unresolved_resolution),
            CompletionOutcome::Success
        );
        let complete = SpecResolution {
            requested: vec!["specs/26".to_string()],
            inlined: vec![InlinedSpec {
                reference: "specs/26".to_string(),
                text: "body".to_string(),
                tokens: 1,
            }],
            unresolved: Vec::new(),
        };
        assert_eq!(
            classify_completion(caveated, &complete),
            CompletionOutcome::Success
        );
    }
}
