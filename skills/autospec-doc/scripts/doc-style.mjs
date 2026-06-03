// doc-style.mjs — palette resolution + mermaid theme injection (issue #920, spec §D4)
//
// Single source of truth for the light-blue palette preset.
// ALL six hex constants live ONLY here — no other file may hardcode them.
// scripts/validate.sh enforces single-source via check_palette_single_source().
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
    const label = steps[i].replace(/"/g, "'").slice(0, 80);
    lines.push(`    ${nodeId}["${label}"]`);
    if (i > 0) {
      lines.push(`    S${i} --> ${nodeId}`);
    }
  }
  return `${init}\n${lines.join('\n')}`;
}

/**
 * Build a themed mermaid flowchart from decision-rule language in the body.
 * Falls back to a simple two-branch decision node.
 * @param {string} body
 * @param {string} heading
 * @returns {string}
 */
function buildDecisionFlowchart(body, heading) {
  const init = mermaidInit();
  const title = heading.replace(/"/g, "'").slice(0, 60);
  // Extract a simple if/otherwise pair if present.
  const ifMatch = body.match(/\bif\b[^,.]*?[,.]?([^.]*)/i);
  const otherwiseMatch = body.match(/\b(otherwise|else|when not)\b[^,.]*/i);
  const lines = ['flowchart TD'];
  lines.push(`    Start["${title}"]`);
  lines.push(`    Cond{Condition?}`);
  lines.push(`    Start --> Cond`);
  if (ifMatch) {
    const yes = ifMatch[0].replace(/^if\s*/i, '').replace(/"/g, "'").slice(0, 60);
    lines.push(`    Cond -->|Yes| Yes["${yes}"]`);
  } else {
    lines.push(`    Cond -->|Yes| Yes["True branch"]`);
  }
  if (otherwiseMatch) {
    const no = otherwiseMatch[0].replace(/^(otherwise|else|when not)\s*/i, '').replace(/"/g, "'").slice(0, 60);
    lines.push(`    Cond -->|No| No["${no}"]`);
  } else {
    lines.push(`    Cond -->|No| No["False branch"]`);
  }
  return `${init}\n${lines.join('\n')}`;
}

/**
 * Build a themed mermaid state diagram from state-machine language in the body.
 * @param {string} body
 * @param {string} heading
 * @returns {string}
 */
function buildStateDiagram(body, heading) {
  const init = mermaidInit();
  // Try to extract "A → B" or "A -> B" transitions.
  const transPattern = /(\w[\w\s-]*)[\s]*(?:→|->)[\s]*([\w][\w\s-]*)/g;
  const transitions = [];
  let m;
  while ((m = transPattern.exec(body)) !== null) {
    transitions.push({ from: m[1].trim(), to: m[2].trim() });
  }
  if (transitions.length === 0) {
    // Fallback: generic two-state diagram
    const title = heading.replace(/"/g, "'").slice(0, 40);
    return `${init}\nstateDiagram-v2\n    [*] --> Active\n    Active --> Done\n    Done --> [*]`;
  }
  const lines = ['stateDiagram-v2'];
  // First transition from gets [*] start
  lines.push(`    [*] --> ${mermaidId(transitions[0].from)}`);
  for (const t of transitions) {
    lines.push(`    ${mermaidId(t.from)} --> ${mermaidId(t.to)}`);
  }
  // Last to gets [*] end
  lines.push(`    ${mermaidId(transitions[transitions.length - 1].to)} --> [*]`);
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

  // 2. State-machine transitions.
  if (/(?:→|->)/.test(body)) {
    return buildStateDiagram(body, heading);
  }

  // 3. Decision / rule language.
  return buildDecisionFlowchart(body, heading);
}
