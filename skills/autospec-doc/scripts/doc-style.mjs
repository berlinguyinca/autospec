// doc-style.mjs — palette resolution + mermaid theme injection (issue #920, spec §D4)
//
// Single source of truth for the light-blue palette preset.
// ALL six hex constants live ONLY here — no other file may hardcode them.
// autospec validate enforces single-source via check_palette_single_source().
//
// Exports:
//   PALETTE          — { background, primary, secondary, accent, line, text }
//   mermaidInit()    — %%{init: {'theme':'base','themeVariables':{...}}}%% block
//   resolvePalette(presetName) → PALETTE | null
//   mermaidInitForPreset(presetName) → string | null
//   isLogicFlowSection(section) → boolean
//   generateExplainerDiagram(section) → string | null

// ── Palette preset: light-blue ────────────────────────────────────────────────
// Pinned hexes per spec §D4.  DO NOT replicate these values in any other file.
export const PALETTE = Object.freeze({
  background: '#E3F2FD',
  primary:    '#90CAF9',
  secondary:  '#BBDEFB',
  accent:     '#1E88E5',
  line:       '#64B5F6',
  text:       '#0D2C45',
});

// ── Preset registry ───────────────────────────────────────────────────────────

const PRESETS = {
  'light-blue': PALETTE,
};

/**
 * Resolve a named palette preset.
 * @param {string} presetName
 * @returns {typeof PALETTE | null}  null when the preset is unknown.
 */
export function resolvePalette(presetName) {
  return PRESETS[presetName] ?? null;
}

// ── Mermaid theme injection ───────────────────────────────────────────────────

/**
 * Build the %%{init}%% front-matter block that injects the light-blue palette
 * into any mermaid diagram rendered with theme:base.
 *
 * The returned string is a single line (no trailing newline) suitable for
 * prepending directly before the diagram type declaration, e.g.:
 *
 *   %%{init: {'theme':'base','themeVariables':{...}}}%%
 *   flowchart LR
 *     A --> B
 *
 * @returns {string}
 */
export function mermaidInit() {
  const vars = {
    background:       PALETTE.background,
    primaryColor:     PALETTE.primary,
    secondaryColor:   PALETTE.secondary,
    tertiaryColor:    PALETTE.secondary,
    primaryBorderColor: PALETTE.accent,
    lineColor:        PALETTE.line,
    textColor:        PALETTE.text,
    nodeBorder:       PALETTE.accent,
    clusterBkg:       PALETTE.secondary,
    titleColor:       PALETTE.text,
  };
  const themeVarsJson = JSON.stringify(vars);
  return `%%{init: {"theme":"base","themeVariables":${themeVarsJson}}}%%`;
}

/**
 * Emit the %%{init}%% block for a named preset.
 * Returns null when the preset is unknown.
 *
 * @param {string} presetName
 * @returns {string | null}
 */
export function mermaidInitForPreset(presetName) {
  if (presetName === 'light-blue') return mermaidInit();
  const pal = resolvePalette(presetName);
  if (!pal) return null;
  // For future presets, build the same shape with their values.
  const vars = {
    background:       pal.background,
    primaryColor:     pal.primary,
    secondaryColor:   pal.secondary,
    tertiaryColor:    pal.secondary,
    primaryBorderColor: pal.accent,
    lineColor:        pal.line,
    textColor:        pal.text,
    nodeBorder:       pal.accent,
    clusterBkg:       pal.secondary,
    titleColor:       pal.text,
  };
  return `%%{init: {"theme":"base","themeVariables":${JSON.stringify(vars)}}}%%`;
}

// ── Algorithm-explainer heuristics ───────────────────────────────────────────

// Patterns that indicate a section describes logic, flows, decisions, or state machines.
const LOGIC_FLOW_PATTERNS = [
  // Ordered steps: "1. …", "2. …" (at least two numbered items)
  /^\d+\.\s+\S/m,
  // Decision language
  /\b(if|otherwise|else|when|unless|given that|in case)\b/i,
  // State transitions (arrow operators or state-machine keywords)
  /(?:→|->)|(?:\b(?:state|transition|states:|phases:)\b)/i,
  // Explicit flow words
  /\b(decision|rule|algorithm|flow|workflow|pipeline|process|guard|route|branch)\b/i,
];

/**
 * Returns true when a spec section describes logic/flows and therefore warrants
 * an algorithm explainer diagram (spec §D4).
 *
 * Heuristic: the section body matches at least one logic/flow pattern AND has
 * sufficient length (≥ 40 chars) to be more than a stub.
 *
 * @param {{ heading: string, body: string }} section
 * @returns {boolean}
 */
export function isLogicFlowSection(section) {
  const body = (section.body || '').trim();
  if (body.length < 40) return false;
  return LOGIC_FLOW_PATTERNS.some(re => re.test(body));
}

// ── Diagram generation helpers ────────────────────────────────────────────────

/**
 * Truncate a label at a word boundary, appending a single ellipsis char when cut.
 *
 * When `text.length <= max` the text is returned unchanged. Otherwise the text is
 * cut at the last word boundary at or before `max` and `…` is appended. Never cuts
 * mid-word — but if the very first word alone exceeds `max`, it is hard-cut.
 *
 * @param {string} text
 * @param {number} max
 * @returns {string}
 */
export function truncateLabel(text, max) {
  const s = String(text);
  if (s.length <= max) return s;
  // Find the last whitespace at or before `max`.
  const slice = s.slice(0, max);
  const lastSpace = slice.search(/\s\S*$/); // index of the last whitespace run start
  if (lastSpace > 0) {
    return s.slice(0, lastSpace).replace(/\s+$/, '') + '…';
  }
  // First word alone exceeds max → hard-cut.
  return slice + '…';
}

/**
 * Sanitize a label for use as a mermaid node ID.
 * @param {string} label
 * @returns {string}
 */
function mermaidId(label) {
  return label.replace(/[^a-zA-Z0-9]/g, '_').replace(/^_+/, '').replace(/_+$/, '') || 'node';
}

/**
 * Extract ordered-step items from a body string.
 * Returns an array of step strings, or [] if none found.
 * @param {string} body
 * @returns {string[]}
 */
function extractOrderedSteps(body) {
  const steps = [];
  for (const line of body.split('\n')) {
    const m = line.match(/^\d+\.\s+(.+)/);
    if (m) steps.push(m[1].trim());
  }
  return steps;
}

/**
 * Build a themed mermaid flowchart from a list of step labels.
 * @param {string[]} steps
 * @returns {string}  complete mermaid block (init header + flowchart)
 */
function buildFlowchart(steps) {
  const init = mermaidInit();
  const lines = ['flowchart TD'];
  for (let i = 0; i < steps.length; i++) {
    const nodeId = `S${i + 1}`;
    const label = truncateLabel(steps[i].replace(/"/g, "'"), 80);
    lines.push(`    ${nodeId}["${label}"]`);
    if (i > 0) {
      lines.push(`    S${i} --> ${nodeId}`);
    }
  }
  return `${init}\n${lines.join('\n')}`;
}

/**
 * Extract a real if/otherwise branch pair from prose, or null when both branches
 * are not present. A decision diagram is only meaningful when BOTH the "if" clause
 * and an "otherwise/else/when not" clause yield non-empty text.
 *
 * @param {string} body
 * @returns {{ yes: string, no: string } | null}
 */
function extractDecisionBranches(body) {
  // "if <clause>," up to the first comma or period (the condition/true action).
  const ifMatch = body.match(/\bif\b\s+([^,.;]+)/i);
  // "otherwise/else/when not <clause>" up to the first comma or period.
  const elseMatch = body.match(/\b(?:otherwise|else|when not)\b\s+([^,.;]+)/i);
  if (!ifMatch || !elseMatch) return null;
  const yes = ifMatch[1].trim();
  const no = elseMatch[1].trim();
  if (!yes || !no) return null;
  return { yes, no };
}

/**
 * Build a themed mermaid decision flowchart from an extracted branch pair.
 * Only call this when extractDecisionBranches() returned a non-null pair.
 *
 * @param {{ yes: string, no: string }} branches
 * @param {string} heading
 * @returns {string}
 */
function buildDecisionFlowchart(branches, heading) {
  const init = mermaidInit();
  const title = truncateLabel(heading.replace(/"/g, "'"), 60);
  const yes = truncateLabel(branches.yes.replace(/"/g, "'"), 60);
  const no = truncateLabel(branches.no.replace(/"/g, "'"), 60);
  const lines = ['flowchart TD'];
  lines.push(`    Start["${title}"]`);
  lines.push(`    Cond{Condition?}`);
  lines.push(`    Start --> Cond`);
  lines.push(`    Cond -->|Yes| Yes["${yes}"]`);
  lines.push(`    Cond -->|No| No["${no}"]`);
  return `${init}\n${lines.join('\n')}`;
}

// A "state" token must be short: ≤ 24 chars and no more than 3 space-separated
// words — anything longer is prose, not a state name.
const MAX_STATE_LEN = 24;

/**
 * Returns true when `token` looks like a clean state name (short, not a sentence
 * fragment) rather than free prose.
 * @param {string} token
 * @returns {boolean}
 */
function isCleanStateToken(token) {
  const t = token.trim();
  if (!t || t.length > MAX_STATE_LEN) return false;
  const words = t.split(/\s+/);
  if (words.length > 3) return false;
  return true;
}

/**
 * Extract a clean ordered state chain from a body containing `A → B → C` style
 * transitions. Returns an array of consecutive {from,to} pairs, deduplicated, or
 * null when the chain is ambiguous/partial (fewer than 2 clean transitions, or
 * any token looks like prose).
 *
 * @param {string} body
 * @returns {{ from: string, to: string }[] | null}
 */
function extractStateChain(body) {
  // Find the longest contiguous run of `token (→|->) token (→|->) …`.
  const chainRe = /([\w][\w-]*(?:\s+[\w][\w-]*){0,2})(?:\s*(?:→|->)\s*([\w][\w-]*(?:\s+[\w][\w-]*){0,2}))+/g;
  let best = null;
  let cm;
  while ((cm = chainRe.exec(body)) !== null) {
    const segment = cm[0];
    // Split the matched segment on arrows into individual state tokens.
    const tokens = segment.split(/\s*(?:→|->)\s*/).map(s => s.trim());
    if (!best || tokens.length > best.length) best = tokens;
  }
  if (!best || best.length < 3) {
    // Need ≥ 3 tokens → ≥ 2 transitions.
    return null;
  }
  // Every token must be a clean state name.
  if (!best.every(isCleanStateToken)) return null;
  // Build consecutive pairs and deduplicate.
  const seen = new Set();
  const pairs = [];
  for (let i = 0; i < best.length - 1; i++) {
    const from = best[i];
    const to = best[i + 1];
    const key = `${from}\u0000${to}`;
    if (seen.has(key)) continue;
    seen.add(key);
    pairs.push({ from, to });
  }
  if (pairs.length < 2) return null;
  return pairs;
}

/**
 * Build a themed mermaid state diagram from an extracted transition chain.
 * Only call this when extractStateChain() returned a non-null array.
 *
 * @param {{ from: string, to: string }[]} pairs
 * @returns {string}
 */
function buildStateDiagram(pairs) {
  const init = mermaidInit();
  const lines = ['stateDiagram-v2'];
  // First transition from gets [*] start.
  lines.push(`    [*] --> ${mermaidId(pairs[0].from)}`);
  for (const t of pairs) {
    lines.push(`    ${mermaidId(t.from)} --> ${mermaidId(t.to)}`);
  }
  // Last to gets [*] end.
  lines.push(`    ${mermaidId(pairs[pairs.length - 1].to)} --> [*]`);
  return `${init}\n${lines.join('\n')}`;
}

/**
 * Generate a themed mermaid explainer diagram for a logic/flow spec section.
 * Returns null if the section is not a logic/flow section.
 *
 * The diagram type is chosen by heuristic:
 *   - Ordered steps (1. 2. 3. …) → flowchart TD
 *   - State-machine language (→ / ->) → stateDiagram-v2
 *   - Decision language (if/otherwise) → flowchart TD with decision node
 *
 * @param {{ heading: string, body: string }} section
 * @returns {string | null}
 */
export function generateExplainerDiagram(section) {
  if (!isLogicFlowSection(section)) return null;

  const body = section.body || '';
  const heading = section.heading || '';

  // 1. Ordered steps take highest priority.
  const steps = extractOrderedSteps(body);
  if (steps.length >= 2) {
    return buildFlowchart(steps);
  }

  // 2. State-machine transitions — only when a clean ≥2-transition chain exists.
  if (/(?:→|->)/.test(body)) {
    const pairs = extractStateChain(body);
    if (pairs) {
      return buildStateDiagram(pairs);
    }
    // Stray arrow inside prose → no diagram.
    return null;
  }

  // 3. Decision / rule language — only when BOTH branches are extractable.
  const branches = extractDecisionBranches(body);
  if (branches) {
    return buildDecisionFlowchart(branches, heading);
  }

  // No real structure extractable → emit no diagram (avoid fabricated noise).
  return null;
}
