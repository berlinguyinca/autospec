//! Context-class fit at dispatch time (issue #3694).
//!
//! Task cards declare a context requirement, e.g. `context class \`C75\`
//! (tier \`tier-integration\`, **65–82K pack**)`, but `--parallel N` divides
//! the engine's context window, so slots on the same fleet differ by 4×
//! (8 slots → 32,768; 4 → 65,536; 2 → 131,072 out of a 256K window).
//!
//! - [`parse_context_class`] extracts the declared requirement from card text.
//! - [`dispatch`] picks the **largest-context** endpoint whose slot holds the
//!   pack floor; when nothing fits the issue is held with
//!   [`NO_ENDPOINT_LARGE_ENOUGH`] — a queued issue is recoverable, a silently
//!   truncated run is not.
//! - [`ContextGrant`] is the record merged into the run's status file by
//!   [`status_json_with_grant`]; a grant short of the declared floor does not
//!   count as an attempt ([`ContextGrant::counts_as_attempt`]), and
//!   [`redispatch`] excludes the endpoint class that already lost.
//!
//! Everything here is pure: callers do the I/O with the returned plan.

use serde::Serialize;

/// Hold code returned when no endpoint can hold the declared pack floor.
pub const NO_ENDPOINT_LARGE_ENOUGH: &str = "NO-ENDPOINT-LARGE-ENOUGH";

/// A context class declared by a task card, e.g. `C75` with a 65–82K pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextClass {
    /// Class name as declared on the card (e.g. `C75`).
    pub name: String,
    /// Tier the class belongs to, when the card names one.
    pub tier: Option<String>,
    /// Lower bound of the pack, in tokens (decimal kilo: `65K` = 65,000).
    pub pack_min_tokens: u32,
    /// Upper bound of the pack, in tokens.
    pub pack_max_tokens: u32,
}

impl ContextClass {
    /// The context a slot must hold for the card to fit.
    pub fn floor_tokens(&self) -> u32 {
        self.pack_min_tokens
    }
}

/// An endpoint as the dispatcher sees it: `--parallel N` divides its context
/// window across `N` slots, and the output reservation reduces each further.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Endpoint name (e.g. from the endpoint file).
    pub name: String,
    /// The engine's full context window (`-c`), in tokens.
    pub context_window_tokens: u32,
    /// Slot count (`--parallel N`), 1 = no division.
    pub parallel_slots: u32,
    /// Tokens reserved for output (`max_tokens`), subtracted when estimating
    /// the working context.
    pub output_reserve_tokens: u32,
}

impl Endpoint {
    /// Context available per slot before the output reservation.
    pub fn context_per_slot(&self) -> u32 {
        self.context_window_tokens / self.parallel_slots.max(1)
    }

    /// Context left for the prompt+files after reserving output room.
    pub fn working_context_per_slot(&self) -> u32 {
        self.context_per_slot()
            .saturating_sub(self.output_reserve_tokens)
    }

    /// Whether a single slot on this endpoint can hold the declared pack.
    pub fn fits(&self, class: &ContextClass) -> bool {
        self.context_per_slot() >= class.pack_min_tokens
    }
}

/// The outcome of a dispatch decision: grant the card to one endpoint, or
/// hold the issue with a machine-readable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchVerdict {
    /// Dispatch the card to `endpoint`, and write `record` to the status file.
    Grant(DispatchGrant),
    /// Nothing fits. `code` is [`NO_ENDPOINT_LARGE_ENOUGH`] for context fit;
    /// the issue stays queued rather than running truncated.
    Hold {
        code: &'static str,
        rationale: String,
    },
}

/// A granted dispatch: the chosen endpoint plus the record to persist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchGrant {
    /// The endpoint the card is dispatched to.
    pub endpoint: Endpoint,
    /// The grant record that goes into the run's status file.
    pub record: ContextGrant,
}

/// The granted context, recorded under `"context"` next to the declared class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextGrant {
    /// Declared class name from the card.
    pub declared_class: String,
    /// Declared tier from the card, if any.
    pub declared_tier: Option<String>,
    /// Declared pack floor, in tokens.
    pub declared_pack_min_tokens: u32,
    /// Declared pack ceiling, in tokens.
    pub declared_pack_max_tokens: u32,
    /// Endpoint the run was granted.
    pub endpoint: String,
    /// Context per slot actually granted, in tokens.
    pub granted_context_tokens: u32,
    /// Granted context minus the output reservation, in tokens.
    pub granted_working_context_tokens: u32,
}

impl ContextGrant {
    /// Build the record for a dispatch of `class` to `endpoint`.
    pub fn for_dispatch(class: &ContextClass, endpoint: &Endpoint) -> Self {
        Self {
            declared_class: class.name.clone(),
            declared_tier: class.tier.clone(),
            declared_pack_min_tokens: class.pack_min_tokens,
            declared_pack_max_tokens: class.pack_max_tokens,
            endpoint: endpoint.name.clone(),
            granted_context_tokens: endpoint.context_per_slot(),
            granted_working_context_tokens: endpoint.working_context_per_slot(),
        }
    }

    /// Whether the grant held at least the declared pack floor.
    pub fn fits_declared_floor(&self) -> bool {
        self.granted_context_tokens >= self.declared_pack_min_tokens
    }

    /// Whether the run counts as an attempt. An under-provisioned run does
    /// not: its failure is evidence about the budget, not the task.
    pub fn counts_as_attempt(&self) -> bool {
        self.fits_declared_floor()
    }
}

/// Choose the endpoint for `class` from `endpoints`: the fitting endpoint
/// with the largest slot context (ties broken by the smaller endpoint name,
/// so the result is independent of fleet ordering). When nothing fits the
/// verdict holds with [`NO_ENDPOINT_LARGE_ENOUGH`] rather than dispatching a
/// truncated run.
pub fn dispatch(class: &ContextClass, endpoints: &[Endpoint]) -> DispatchVerdict {
    let mut best: Option<&Endpoint> = None;
    for endpoint in endpoints {
        if !endpoint.fits(class) {
            continue;
        }
        let better = match best {
            Some(current) => {
                let mine = endpoint.context_per_slot();
                let theirs = current.context_per_slot();
                mine > theirs || (mine == theirs && endpoint.name < current.name)
            }
            None => true,
        };
        if better {
            best = Some(endpoint);
        }
    }
    match best {
        Some(endpoint) => DispatchVerdict::Grant(DispatchGrant {
            endpoint: endpoint.clone(),
            record: ContextGrant::for_dispatch(class, endpoint),
        }),
        None => DispatchVerdict::Hold {
            code: NO_ENDPOINT_LARGE_ENOUGH,
            rationale: hold_rationale(class, endpoints),
        },
    }
}

/// Re-dispatch an under-provisioned run over the fitting endpoints that are
/// **strictly larger** than the grant that already lost; holds with
/// [`NO_ENDPOINT_LARGE_ENOUGH`] when no larger endpoint remains.
pub fn redispatch(
    class: &ContextClass,
    endpoints: &[Endpoint],
    previous: &ContextGrant,
) -> DispatchVerdict {
    let bigger: Vec<Endpoint> = endpoints
        .iter()
        .filter(|e| e.fits(class) && e.context_per_slot() > previous.granted_context_tokens)
        .cloned()
        .collect();
    dispatch(class, &bigger)
}

/// Merge `grant` into the run's status file as a `"context"` object, keeping
/// whatever the runner already wrote; returns the serialised JSON.
pub fn status_json_with_grant(
    existing: Option<&str>,
    grant: &ContextGrant,
) -> Result<String, String> {
    let mut obj: serde_json::Value = match existing {
        Some(text) => serde_json::from_str(text)
            .map_err(|e| format!("existing status is not valid JSON: {e}"))?,
        None => serde_json::json!({}),
    };
    let obj = obj
        .as_object_mut()
        .ok_or_else(|| "existing status is not a JSON object".to_string())?;
    let grant_value = serde_json::to_value(grant).map_err(|e| e.to_string())?;
    obj.insert("context".to_string(), grant_value);
    serde_json::to_string_pretty(obj).map_err(|e| e.to_string())
}

fn hold_rationale(class: &ContextClass, endpoints: &[Endpoint]) -> String {
    let largest = endpoints
        .iter()
        .map(|e| e.context_per_slot())
        .max()
        .unwrap_or(0);
    format!(
        "declared {} pack floor {} tokens; largest slot available is {} tokens",
        class.name, class.pack_min_tokens, largest
    )
}

/// Parse the declared context class from task-card text. Lenient about
/// markdown, case, and dash style; strict about the class name and the pack
/// floor — it returns `None` rather than guess. `K` is decimal kilo (×1,000)
/// and may attach to either end of a range (`65–82K`, `65K–82K`).
pub fn parse_context_class(text: &str) -> Option<ContextClass> {
    let name = class_name(text)?;
    let (pack_min_tokens, pack_max_tokens) = pack_range(text)?;
    Some(ContextClass {
        name,
        tier: tier_name(text),
        pack_min_tokens,
        pack_max_tokens,
    })
}

/// All maximal ASCII-alphanumeric runs with their byte positions.
fn ident_runs(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        if !c.is_ascii_alphanumeric() {
            if start < i {
                out.push((start, &text[start..i]));
            }
            start = i + c.len_utf8();
        }
    }
    if start < text.len() {
        out.push((start, &text[start..]));
    }
    out
}

/// A class-shaped token: starts with a letter and contains a digit (`C75`).
fn is_class_token(token: &str) -> bool {
    token
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && token.chars().any(|c| c.is_ascii_digit())
}

/// The class-shaped token after a "context class" marker, else the first one.
fn class_name(text: &str) -> Option<String> {
    let runs = ident_runs(text);
    for i in 0..runs.len().saturating_sub(2) {
        if !runs[i].1.eq_ignore_ascii_case("context")
            || !runs[i + 1].1.eq_ignore_ascii_case("class")
        {
            continue;
        }
        if is_class_token(runs[i + 2].1) {
            return Some(runs[i + 2].1.to_string());
        }
    }
    runs.iter()
        .find(|(_, token)| is_class_token(token))
        .map(|(_, token)| token.to_string())
}

/// The hyphenated tier named after the word `tier` (e.g. `tier-integration`).
fn tier_name(text: &str) -> Option<String> {
    for (pos, token) in ident_runs(text) {
        if !token.eq_ignore_ascii_case("tier") {
            continue;
        }
        let rest = &text[pos + token.len()..];
        let mut j = 0;
        while matches!(rest.as_bytes().get(j), Some(b' ' | b'\t' | b'*' | b'`')) {
            j += 1;
        }
        let tail = &rest[j..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(tail.len());
        let candidate = &tail[..end];
        if !candidate.is_empty()
            && candidate.starts_with(|c: char| c.is_ascii_alphabetic())
            && candidate.contains('-')
        {
            return Some(candidate.to_string());
        }
    }
    None
}

/// The pack range in tokens: the first standalone number or numeric range in
/// the text (numbers glued to letters, e.g. the `75` inside `C75`, are
/// class tokens, not pack sizes).
fn pack_range(text: &str) -> Option<(u32, u32)> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if is_standalone_digit(&chars, i) {
            if let Some(range) = pack_at(&chars, i) {
                return Some(range);
            }
        }
        i += 1;
    }
    None
}

/// True at a number not glued to an identifier: `75` inside `C75` is a class token.
fn is_standalone_digit(chars: &[char], i: usize) -> bool {
    chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric())
}

/// Parse the number at `i`, an optional `K` unit, and an optional range with
/// a second number; a unit on either end covers both (`65–82K` = 65,000–82,000).
fn pack_at(chars: &[char], i: usize) -> Option<(u32, u32)> {
    let (first, mut j) = number_at(chars, i)?;
    skip_ws_star(&mut j, chars);
    let first_k = has_k_unit(chars, j);
    if first_k {
        j += 1;
        skip_ws_star(&mut j, chars);
    }
    if !is_range_sep_or_to(chars, j) {
        let value = if first_k {
            first.saturating_mul(1_000)
        } else {
            first
        };
        return Some((value, value));
    }
    let mut k = j + if is_word_to(chars, j) { 2 } else { 1 };
    skip_ws_star(&mut k, chars);
    let (second, m) = number_at(chars, k)?;
    let second_k = has_k_unit(chars, m);
    let unit = if first_k || second_k { 1_000 } else { 1 };
    let lo = first.saturating_mul(unit);
    let hi = second.saturating_mul(unit);
    Some(if lo <= hi { (lo, hi) } else { (hi, lo) })
}

/// The run of digits at `i` as a value, plus the index just past it.
fn number_at(chars: &[char], i: usize) -> Option<(u32, usize)> {
    let mut j = i;
    while j < chars.len() && chars[j].is_ascii_digit() {
        j += 1;
    }
    let value: u32 = chars[i..j].iter().collect::<String>().parse().ok()?;
    Some((value, j))
}

fn has_k_unit(chars: &[char], j: usize) -> bool {
    j < chars.len() && matches!(chars[j], 'K' | 'k')
}

fn is_range_sep_or_to(chars: &[char], j: usize) -> bool {
    j < chars.len() && (is_range_sep(chars[j]) || is_word_to(chars, j))
}

fn skip_ws_star(i: &mut usize, chars: &[char]) {
    while *i < chars.len() && (chars[*i].is_whitespace() || chars[*i] == '*') {
        *i += 1;
    }
}

fn is_range_sep(c: char) -> bool {
    matches!(c, '-' | '~' | '\u{2013}' | '\u{2014}')
}

fn is_word_to(chars: &[char], j: usize) -> bool {
    j > 0
        && !chars[j - 1].is_ascii_alphanumeric()
        && chars.get(j).is_some_and(|c| *c == 't')
        && chars.get(j + 1).is_some_and(|c| *c == 'o')
        && !chars.get(j + 2).is_some_and(|c| c.is_ascii_alphanumeric())
}
