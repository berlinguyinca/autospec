#!/usr/bin/env bash
# skills/autospec-shared/scripts/bundle-static-context.sh — Emit static prompt prefix for LLM dispatch.
#
# Usage:
#   bundle-static-context.sh --role implementer|reviewer|decomposer|classifier
#                            [--issue-labels "label1,label2,..."]
#
# Output (stdout): static blob framed by <!-- CACHE BOUNDARY --> markers.
#
# Composition order (implementer role — D3 prefix slim):
#   --- inside the CACHE BOUNDARY (byte-stable prefix, safe to cache) ---
#   1. implementer-contract.md (curated <=24KB extract — NOT the 70KB SKILL.md)
#   2. AGENTS.md ## Implementation-quality contract section (canonical RULE_ID)
#   --- below the closing CACHE BOUNDARY (per-issue, NOT cached) ---
#   3. Tag-filtered saved-memory files
#   4. Lockstep paragraph
#   5. Role-specific implementer scaffolding
#
# Composition order (reviewer/decomposer/classifier roles — unchanged):
#   1. SKILL.md (role's skill, verbatim)
#   2. AGENTS.md (verbatim)
#   3. RULE_ID table (extracted from AGENTS.md)
#   4. Tag-filtered saved-memory files
#   5. Lockstep paragraph
#   6. Role-specific scaffolding
#
# Implementer prefix caching: the launch path SHOULD pass everything ABOVE the
# closing <!-- CACHE BOUNDARY --> with cache_control: {type: ephemeral} where the
# harness supports it. The prefix is byte-stable across issues because all
# per-issue material (memory, scaffolding, dynamic suffix) lives below the marker.
#
# Environment overrides (for testing):
#   AUTOSPEC_REPO_ROOT    — repo root (default: dirname of this script / 3 levels up)
#   AUTOSPEC_MEMORY_DIR   — path to memory files
#   AUTOSPEC_SCRIPTS_DIR  — path to scripts dir (for memory-tags.yml)
#
# Idempotent + deterministic: same inputs → byte-identical output.
#
# Exit codes:
#   0  Success
#   1  Argument/file error or unknown role
#
# Compatible with bash 3.2+

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# Resolve repo root: scripts live at skills/autospec-shared/scripts/ — 3 levels up
REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"

HELP_TEXT='Usage:
  bundle-static-context.sh --role implementer|reviewer|decomposer|classifier
                            [--issue-labels "label1,label2,..."]
  bundle-static-context.sh --help

Emit static LLM prompt prefix framed by <!-- CACHE BOUNDARY --> markers.
Composition: SKILL.md + AGENTS.md + RULE_ID table + tag-filtered memory + lockstep + role scaffolding.'

ROLE=""
ISSUE_LABELS=""

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h)
      printf '%s\n' "$HELP_TEXT"
      exit 0
      ;;
    --role)
      if [ $# -lt 2 ]; then printf 'bundle-static-context.sh: --role requires an argument\n' >&2; exit 1; fi
      ROLE="$2"
      shift 2
      ;;
    --issue-labels)
      if [ $# -lt 2 ]; then printf 'bundle-static-context.sh: --issue-labels requires an argument\n' >&2; exit 1; fi
      ISSUE_LABELS="$2"
      shift 2
      ;;
    -*)
      printf 'bundle-static-context.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      printf 'bundle-static-context.sh: unexpected argument: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

if [ -z "$ROLE" ]; then
  printf '%s\n' "$HELP_TEXT" >&2
  exit 1
fi

# Validate role
case "$ROLE" in
  implementer|reviewer|decomposer|classifier) ;;
  *)
    printf 'bundle-static-context.sh: unknown role: %s (must be implementer|reviewer|decomposer|classifier)\n' "$ROLE" >&2
    exit 1
    ;;
esac

# ── resolve paths ─────────────────────────────────────────────────────────────

WHOAMI=$(whoami 2>/dev/null || echo "$(id -un)")
MEMORY_DIR="${AUTOSPEC_MEMORY_DIR:-$HOME/.claude/projects/-Users-${WHOAMI}-IdeaProjects-autospec/memory}"
SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
# Allow explicit override for testing; fall back to scripts dir
MANIFEST="${AUTOSPEC_MANIFEST:-$SCRIPTS_DIR/memory-tags.yml}"
AGENTS_MD="$REPO_ROOT/AGENTS.md"
# D3: the implementer role injects this curated contract instead of the 70KB SKILL.md.
IMPLEMENTER_CONTRACT="$REPO_ROOT/skills/autospec-run/prompts/implementer-contract.md"

# Role-to-skill mapping. The implementer role no longer injects a SKILL.md
# (D3 prefix slim) — it injects implementer-contract.md instead.
SKILL_MD=""
case "$ROLE" in
  implementer) SKILL_MD="" ;;
  reviewer)    SKILL_MD="$REPO_ROOT/skills/autospec-run/SKILL.md" ;;
  decomposer)  SKILL_MD="$REPO_ROOT/skills/autospec-define/SKILL.md" ;;
  classifier)  SKILL_MD="$REPO_ROOT/skills/autospec-classify/SKILL.md" ;;
esac

# Validate required files
if [ ! -f "$AGENTS_MD" ]; then
  printf 'bundle-static-context.sh: AGENTS.md not found: %s\n' "$AGENTS_MD" >&2
  exit 1
fi
if [ "$ROLE" = "implementer" ]; then
  if [ ! -f "$IMPLEMENTER_CONTRACT" ]; then
    printf 'bundle-static-context.sh: implementer-contract.md not found: %s\n' "$IMPLEMENTER_CONTRACT" >&2
    exit 1
  fi
elif [ ! -f "$SKILL_MD" ]; then
  printf 'bundle-static-context.sh: SKILL.md not found for role %s: %s\n' "$ROLE" "$SKILL_MD" >&2
  exit 1
fi

# ── tag-filtered memory selection ────────────────────────────────────────────

# Normalize issue labels to space-separated lowercase tokens.
# Also split each label on ':' to generate sub-tokens, so that
# "skill:autospec-run" matches both "skill" and "autospec-run" tags.
_raw_tokens=$(printf '%s' "$ISSUE_LABELS" | tr ',' ' ' | tr '[:upper:]' '[:lower:]')
_colon_tokens=$(printf '%s' "$_raw_tokens" | tr ':' ' ')
issue_tokens=" ${_raw_tokens} ${_colon_tokens} "

# Build list of matched memory file paths from manifest
matched_memory=""
if [ -f "$MANIFEST" ] && [ -d "$MEMORY_DIR" ]; then
  current_file=""
  while IFS= read -r line; do
    # Detect filename line: feedback_foo.md:
    if printf '%s' "$line" | grep -qE '^feedback_[^:]+\.md:'; then
      current_file=$(printf '%s' "$line" | sed 's/:.*//')
      continue
    fi
    # Detect tags line:   tags: [a, b, c]
    if printf '%s' "$line" | grep -qE '^[[:space:]]+tags:' && [ -n "$current_file" ]; then
      tags_raw=$(printf '%s' "$line" | sed 's/.*\[//; s/\].*//')
      current_path="$MEMORY_DIR/$current_file"
      match=0
      for tag in $(printf '%s' "$tags_raw" | tr ',' '\n' | tr -d ' []"'"'" | tr '[:upper:]' '[:lower:]'); do
        if printf '%s' "$issue_tokens" | grep -qw "$tag"; then
          match=1
          break
        fi
      done
      if [ "$match" -eq 1 ] && [ -f "$current_path" ]; then
        matched_memory="${matched_memory}${current_path}
"
      fi
      current_file=""
    fi
  done < "$MANIFEST"
fi

# ── emit helpers ──────────────────────────────────────────────────────────────

# Emit tag-filtered saved-memory files (per-issue; must live BELOW the closing
# CACHE BOUNDARY so the implementer prefix stays byte-stable across issues).
emit_memory() {
  printf '## Project rules (saved memory)\n\n'
  if [ -z "$matched_memory" ]; then
    printf '(No memory files matched the issue labels.)\n\n'
  else
    printf '%s' "$matched_memory" | while IFS= read -r mem_file; do
      [ -n "$mem_file" ] || continue
      [ -f "$mem_file" ] || continue
      # Strip YAML frontmatter (---...---) if present
      awk '
        BEGIN { in_front=0; done_front=0 }
        /^---$/ && !done_front && NR==1 { in_front=1; next }
        /^---$/ && in_front { in_front=0; done_front=1; next }
        !in_front { print }
      ' "$mem_file"
      printf '\n'
    done
  fi
}

# Emit the lockstep paragraph (static, but kept below the boundary alongside the
# other per-role scaffolding for the implementer role).
emit_lockstep() {
  printf '## Lockstep rules\n\n'
  printf 'All SKILL.md edits must be byte-identically mirrored to codex/prompt.md and opencode/agent.md.\n'
  printf 'Run scripts/validate.sh to confirm lockstep compliance before every push.\n'
  printf 'Renaming any prose section heading in SKILL.md requires updating scripts/validate.sh named-content checks.\n'
  printf '\n'
}

# Emit role-specific scaffolding (per-issue acting instructions).
emit_scaffolding() {
  # Output discipline (all roles): a subagent's final message flows back into the
  # orchestrator's context, so verbose returns bloat context + burn tokens. Keep
  # returns high-density.
  printf '## Output discipline\n\n'
  printf 'Your final message returns to the orchestrator — keep it high-density. Return only conclusions, file:line references, and uncertainty; no preamble, prompt restatement, or end-summary. Make minimal diffs — never rewrite a whole file for a small change. Use markdown only where it compresses (tables); otherwise terse prose.\n'
  printf '\n'
  case "$ROLE" in
    implementer)
      printf '## Implementer scaffolding\n\n'
      printf 'You are implementing a GitHub issue in a git worktree off origin/main.\n'
      printf 'TDD discipline: write failing tests first, then implement, then make tests pass.\n'
      printf 'Use conventional commits (feat:/fix:/test:/docs:/refactor:). NEVER bypass hooks. NEVER amend.\n'
      printf 'Keep heartbeat updated at each major step: claimed → worktree_ready → tests_started → tests_passed → pr_created → reviewed → merged.\n'
      printf '\n'
      ;;
    reviewer)
      printf '## Reviewer scaffolding\n\n'
      printf 'You are performing fused guardian + LGTM review of a GitHub PR.\n'
      printf 'Part 1 — Guardian: apply RULE_ID table to the diff. Collect findings as RULE_ID:<path>:<line>: <desc>.\n'
      printf 'Part 2 — LGTM: check correctness, edge cases, missing tests, AGENTS.md compliance.\n'
      printf 'Verdict: if zero blocking findings → return only the token LGTM. Otherwise return numbered findings list.\n'
      printf '\n'
      ;;
    decomposer)
      printf '## Decomposer scaffolding\n\n'
      printf 'You are decomposing a feature spec into GitHub issues for autonomous implementation.\n'
      printf 'Each issue must be self-contained, sized for 32B-class local LLMs (48 GB Macs), with one primary smoke test.\n'
      printf '\n'
      ;;
    classifier)
      printf '## Classifier scaffolding\n\n'
      printf 'You are classifying GitHub issues with model-fit labels (ctx:* and reasoning:*).\n'
      printf 'Ordinals: ctx: 32k < 64k < 120k; reasoning: shallow < medium < deep.\n'
      printf '\n'
      ;;
  esac
}

# Emit the repo's adopted design language (DESIGN.md at repo root), if present, so
# the implementer honors design tokens/constraints instead of hardcoding values.
# Repo-wide and byte-stable across issues, so it lives inside the cache boundary.
emit_design() {
  design_md="$REPO_ROOT/DESIGN.md"
  [ -f "$design_md" ] || return 0
  printf '## Design language (DESIGN.md — honor these tokens; do not hardcode)\n\n'
  cat "$design_md"
  printf '\n'
}

# ── emit output ───────────────────────────────────────────────────────────────

if [ "$ROLE" = "implementer" ]; then
  # D3 prefix slim: the implementer prefix is byte-stable across issues. Only
  # the curated contract + AGENTS.md quality-contract section live inside the
  # cache boundary; per-issue memory + scaffolding go BELOW the closing marker.

  printf '<!-- CACHE BOUNDARY -->\n'

  # 1. Curated implementer contract (NOT the 70KB SKILL.md).
  printf '## Implementer contract\n\n'
  cat "$IMPLEMENTER_CONTRACT"
  printf '\n'

  # 2. AGENTS.md ## Implementation-quality contract section (canonical source of
  #    the RULE_ID table). The contract above mirrors it; validate.sh flags drift.
  printf '## AGENTS.md — Implementation-quality contract\n\n'
  awk '
    /^## Implementation-quality contract/ { in_sec=1; print; next }
    /^## / && in_sec { exit }
    in_sec { print }
  ' "$AGENTS_MD"
  printf '\n'

  # 3. Adopted design language (DESIGN.md), if the repo has one — repo-wide and
  #    byte-stable, so it stays inside the cache boundary.
  emit_design

  printf '<!-- CACHE BOUNDARY -->\n'

  # Below the boundary: per-issue, NOT part of the cached prefix.
  emit_memory
  emit_lockstep
  emit_scaffolding
else
  # reviewer / decomposer / classifier: unchanged composition (everything inside
  # the boundary; these roles do not yet use the byte-stable-prefix split).
  printf '<!-- CACHE BOUNDARY -->\n'

  # 1. SKILL.md verbatim
  printf '## SKILL.md (%s role)\n\n' "$ROLE"
  cat "$SKILL_MD"
  printf '\n'

  # 2. AGENTS.md verbatim
  printf '## AGENTS.md\n\n'
  cat "$AGENTS_MD"
  printf '\n'

  # 3. RULE_ID table extracted from AGENTS.md
  printf '## RULE_ID table\n\n'
  awk '
    /^### RULE_ID table/ { in_table=1; next }
    /^### / && in_table { exit }
    /^## / && in_table { exit }
    in_table { print }
  ' "$AGENTS_MD"
  printf '\n'

  # 4. Tag-filtered saved-memory files
  emit_memory

  # 5. Lockstep paragraph
  emit_lockstep

  # 6. Role-specific scaffolding
  emit_scaffolding

  printf '<!-- CACHE BOUNDARY -->\n'
fi
