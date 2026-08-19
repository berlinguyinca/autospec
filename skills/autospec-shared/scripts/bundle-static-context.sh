#!/usr/bin/env bash
# skills/autospec-shared/scripts/bundle-static-context.sh — Emit static prompt prefix for LLM dispatch.
#
# Usage:
#   bundle-static-context.sh --role implementer|reviewer|decomposer|classifier
#                            [--issue-labels "label1,label2,..."]
#                            [--static-body <file>]
#
# Output (stdout): static blob framed by <!-- CACHE BOUNDARY --> markers.
#
# Composition order (implementer role — D3 prefix slim):
#   --- inside the CACHE BOUNDARY (byte-stable prefix, safe to cache) ---
#   1. implementer-contract.md (curated <=24KB extract — NOT the 70KB SKILL.md)
#   2. AGENTS.md ## Implementation-quality contract section (canonical RULE_ID)
#   3. Lockstep paragraph
#   4. Role-specific implementer scaffolding
#   5. --static-body file when given (the Phase 4 implementer template)
#   --- below the closing CACHE BOUNDARY (per-issue, NOT cached) ---
#   6. Tag-filtered saved-memory files
#
# Only saved-memory is per-issue: it is label-filtered, so it cannot sit in a
# byte-stable prefix. The lockstep paragraph and the role scaffolding are fixed
# strings, and --static-body is one fixed file, so all three belong above the
# closing marker. Leaving the 8.9k-token Phase 4 template below it meant the
# implementer re-sent it uncached on every dispatch and every retry.
#
# Composition order (reviewer role — prompt-cache reclaim Phase 1 prefix slim):
#   --- inside the CACHE BOUNDARY (byte-stable prefix, safe to cache) ---
#   1. reviewer-contract.md (curated guardian rubric + RULE_ID table — NOT the
#      ~14KB autospec-run/SKILL.md monitor-loop machinery)
#   2. AGENTS.md ## Implementation-quality contract section (canonical RULE_ID)
#   --- below the closing CACHE BOUNDARY (per-issue, NOT cached) ---
#   3. Tag-filtered saved-memory files
#   4. Lockstep paragraph
#   5. Reviewer scaffolding
#
# Composition order (decomposer role — M1 prefix slim):
#   --- inside the CACHE BOUNDARY ---
#   1. decomposer-contract.md (curated Phase 3 extract — NOT the 67KB SKILL.md)
#   2. AGENTS.md ## Issue-quality contract + ## Small-LLM target sections
#   --- below the closing CACHE BOUNDARY (per-issue) ---
#   3. Tag-filtered saved-memory files
#   4. Lockstep paragraph
#   5. Role-specific scaffolding
#
# Composition order (classifier role — M1 prefix slim):
#   --- inside the CACHE BOUNDARY ---
#   1. classifier-contract.md (curated Phase 3.5 extract — NOT the ~18KB SKILL.md)
#   2. AGENTS.md ## Subagent model selection section
#   --- below the closing CACHE BOUNDARY (per-issue) ---
#   3. Tag-filtered saved-memory files
#   4. Lockstep paragraph
#   5. Role-specific scaffolding
#
# Memory injection is capped at AUTOSPEC_MAX_MEMORY_FILES (default 6) — only the
# top-K most-specific tag matches are injected, so memory never busts the budget.
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
# Resolve the root that carries AGENTS.md and the skills/*/prompts contracts.
# install.sh flattens helpers into ~/.autospec/scripts, so "three levels up is
# the repo root" only holds for the in-repo layout. Probing for AGENTS.md keeps
# the in-repo answer identical while giving flat installs a real answer instead
# of /home.
autospec_bsc_resolve_repo_root() {
  _bsc_candidate=""
  if [ -n "${AUTOSPEC_REPO_ROOT:-}" ]; then
    printf '%s\n' "$AUTOSPEC_REPO_ROOT"
    return 0
  fi
  # In-repo layout: skills/autospec-shared/scripts/<this script>.
  _bsc_candidate="$(cd "$SCRIPT_DIR/../../.." 2>/dev/null && pwd)" || _bsc_candidate=""
  if [ -n "$_bsc_candidate" ] && [ -f "$_bsc_candidate/AGENTS.md" ]; then
    printf '%s\n' "$_bsc_candidate"
    return 0
  fi
  # Flat install: the repository the caller is actually working in.
  _bsc_candidate="$(git rev-parse --show-toplevel 2>/dev/null)" || _bsc_candidate=""
  if [ -n "$_bsc_candidate" ] && [ -f "$_bsc_candidate/AGENTS.md" ]; then
    printf '%s\n' "$_bsc_candidate"
    return 0
  fi
  # Flat install outside any repo: the canonical checkout install.sh maintains.
  _bsc_candidate="${AUTOSPEC_HOME:-$HOME/.autospec}/repo"
  if [ -f "$_bsc_candidate/AGENTS.md" ]; then
    printf '%s\n' "$_bsc_candidate"
    return 0
  fi
  return 1
}
REPO_ROOT="$(autospec_bsc_resolve_repo_root)" || REPO_ROOT="$SCRIPT_DIR/../../.."

HELP_TEXT='Usage:
  bundle-static-context.sh --role implementer|reviewer|decomposer|classifier
                            [--issue-labels "label1,label2,..."]
                            [--static-body <file>]
  bundle-static-context.sh --help

Emit static LLM prompt prefix framed by <!-- CACHE BOUNDARY --> markers.
Composition: curated role contract + AGENTS.md RULE_ID table + lockstep +
role scaffolding + --static-body inside the boundary; tag-filtered memory below it.'

ROLE=""
ISSUE_LABELS=""
STATIC_BODY=""

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
    --static-body)
      if [ $# -lt 2 ]; then printf 'bundle-static-context.sh: --static-body requires an argument\n' >&2; exit 1; fi
      # Fail loudly. A silently dropped body dispatches an implementer with the
      # contract but no procedure, which reads like a normal run.
      if [ ! -f "$2" ]; then
        printf 'bundle-static-context.sh: --static-body file not found: %s\n' "$2" >&2
        exit 1
      fi
      STATIC_BODY="$2"
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

# --static-body is emitted only by the implementer branch below. Accepting it for
# another role would silently drop a whole procedure, which is the exact failure
# this option was added to prevent.
if [ -n "$STATIC_BODY" ] && [ "$ROLE" != "implementer" ]; then
  printf 'bundle-static-context.sh: --static-body is only supported for --role implementer (got %s)\n' "$ROLE" >&2
  exit 1
fi

# ── resolve paths ─────────────────────────────────────────────────────────────

# Claude Code names its per-project directory after the absolute repo path with
# the separators rewritten, and the exact rewrite has changed between releases —
# this host carries both `-autospec--claude-worktrees-...` and
# `-go-modules-.worktrees-...` — so probe candidates instead of reproducing it.
#
# The hardcoded `-Users-${WHOAMI}-IdeaProjects-autospec` prefix this replaced could
# only ever match one macOS layout. On Linux the directory never existed, so every
# implementer, decomposer, and classifier dispatch injected nothing and still
# printed the ordinary "(No memory files matched the issue labels.)" line — the
# failure was indistinguishable from "this issue has no relevant memory". The
# committed `docs/memory` store carries the same manifest filenames as the harness
# store, so it is a harness-independent last resort. Their contents are not
# identical, so which one answers is observable — the harness store wins when it
# exists. Note REPO_ROOT is usually a per-session worktree, whose slug has no
# harness directory, so the committed store is what most runs resolve to.
if [ -n "${AUTOSPEC_MEMORY_DIR:-}" ]; then
  MEMORY_DIR="$AUTOSPEC_MEMORY_DIR"
else
  _bsc_slug="$(printf '%s' "$REPO_ROOT" | tr '/' '-')"
  _bsc_harness_dir="$HOME/.claude/projects/${_bsc_slug}/memory"
  # Default to the harness path so a debug dump still names something meaningful
  # when neither candidate exists.
  MEMORY_DIR="$_bsc_harness_dir"
  for _bsc_candidate in "$_bsc_harness_dir" "$REPO_ROOT/docs/memory"; do
    if [ -d "$_bsc_candidate" ]; then
      MEMORY_DIR="$_bsc_candidate"
      break
    fi
  done
  unset _bsc_slug _bsc_harness_dir _bsc_candidate
fi
SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}"
# Allow explicit override for testing; fall back to scripts dir
MANIFEST="${AUTOSPEC_MANIFEST:-$SCRIPTS_DIR/memory-tags.yml}"
AGENTS_MD="$REPO_ROOT/AGENTS.md"
# D3: the implementer role injects this curated contract instead of the 70KB SKILL.md.
IMPLEMENTER_CONTRACT="$REPO_ROOT/skills/autospec-run/prompts/implementer-contract.md"
# Phase 1 (prompt-cache reclaim): the reviewer role injects this curated contract
# instead of the ~14KB autospec-run/SKILL.md.
REVIEWER_CONTRACT="$REPO_ROOT/skills/autospec-run/prompts/reviewer-contract.md"
# M1 (prefix slim): the decomposer role injects this curated Phase 3 contract
# instead of the 67KB autospec-define/SKILL.md.
DECOMPOSER_CONTRACT="$REPO_ROOT/skills/autospec-define/prompts/decomposer-contract.md"
# M1 (prefix slim): the classifier role injects this curated Phase 3.5 contract
# instead of the ~18KB autospec-classify/SKILL.md.
CLASSIFIER_CONTRACT="$REPO_ROOT/skills/autospec-classify/prompts/classifier-contract.md"

# Cap on injected saved-memory files (top-K most-specific tag matches).
AUTOSPEC_MAX_MEMORY_FILES="${AUTOSPEC_MAX_MEMORY_FILES:-6}"

# Role-to-contract mapping. Every role injects a curated *-contract.md (prefix
# slim) instead of its full SKILL.md — implementer/reviewer since D3/Phase 1,
# decomposer/classifier since M1. SKILL_MD is no longer used by any role.
SKILL_MD=""

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
elif [ "$ROLE" = "reviewer" ]; then
  if [ ! -f "$REVIEWER_CONTRACT" ]; then
    printf 'bundle-static-context.sh: reviewer-contract.md not found: %s\n' "$REVIEWER_CONTRACT" >&2
    exit 1
  fi
elif [ "$ROLE" = "decomposer" ]; then
  if [ ! -f "$DECOMPOSER_CONTRACT" ]; then
    printf 'bundle-static-context.sh: decomposer-contract.md not found: %s\n' "$DECOMPOSER_CONTRACT" >&2
    exit 1
  fi
elif [ "$ROLE" = "classifier" ]; then
  if [ ! -f "$CLASSIFIER_CONTRACT" ]; then
    printf 'bundle-static-context.sh: classifier-contract.md not found: %s\n' "$CLASSIFIER_CONTRACT" >&2
    exit 1
  fi
fi

# ── tag-filtered memory selection ────────────────────────────────────────────

# Normalize issue labels to space-separated lowercase tokens.
# Also split each label on ':' to generate sub-tokens, so that
# "skill:autospec-run" matches both "skill" and "autospec-run" tags.
_raw_tokens=$(printf '%s' "$ISSUE_LABELS" | tr ',' ' ' | tr '[:upper:]' '[:lower:]')
_colon_tokens=$(printf '%s' "$_raw_tokens" | tr ':' ' ')
issue_tokens=" ${_raw_tokens} ${_colon_tokens} "

# Build list of matched memory file paths from manifest. Each match is scored by
# how many of its tags hit the issue tokens (more matches = more specific), so we
# can keep only the top-K most-specific files (AUTOSPEC_MAX_MEMORY_FILES).
# Intermediate lines are "<score>\t<path>"; ranked + capped below.
scored_memory=""
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
      match_count=0
      for tag in $(printf '%s' "$tags_raw" | tr ',' '\n' | tr -d ' []"'"'" | tr '[:upper:]' '[:lower:]'); do
        if printf '%s' "$issue_tokens" | grep -qw "$tag"; then
          match_count=$((match_count + 1))
        fi
      done
      if [ "$match_count" -gt 0 ] && [ -f "$current_path" ]; then
        scored_memory="${scored_memory}${match_count}	${current_path}
"
      fi
      current_file=""
    fi
  done < "$MANIFEST"
fi

# Rank by specificity (descending match score), break ties by path for a stable
# byte-identical ordering, then cap to the top-K. Keeps only the path column.
matched_memory=""
if [ -n "$scored_memory" ]; then
  matched_memory=$(printf '%s' "$scored_memory" \
    | grep -v '^$' \
    | sort -t'	' -k1,1nr -k2,2 \
    | head -n "$AUTOSPEC_MAX_MEMORY_FILES" \
    | cut -f2-)
  [ -n "$matched_memory" ] && matched_memory="${matched_memory}
"
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

# Emit the lockstep paragraph. Fixed strings, so it rides inside the cache
# boundary for the implementer role.
emit_lockstep() {
  printf '## Lockstep rules\n\n'
  printf 'All SKILL.md edits must be byte-identically mirrored to codex/prompt.md and opencode/agent.md.\n'
  printf 'Run autospec validate to confirm lockstep compliance before every push.\n'
  printf 'Renaming any prose section heading in SKILL.md requires updating the direct Rust validation catalog.\n'
  printf '\n'
}

# Emit role-specific scaffolding. Fixed per-role strings with no per-issue data,
# despite the "per-issue" label this carried before.
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

# Emit the --static-body file, if given. One fixed file per role, so it belongs
# inside the cache boundary; see the composition note in the header.
emit_static_body() {
  [ -n "$STATIC_BODY" ] || return 0
  cat "$STATIC_BODY"
  printf '\n'
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
  # D3 prefix slim: the implementer prefix is byte-stable across issues, so every
  # fixed section lives inside the cache boundary. Only label-filtered memory,
  # which varies per issue, goes BELOW the closing marker.

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

  # 4-6. Fixed strings and the one fixed body file. Order matches the pre-#3228
  #      output so only memory changes position.
  emit_lockstep
  emit_scaffolding
  emit_static_body

  printf '<!-- CACHE BOUNDARY -->\n'

  # Below the boundary: label-filtered, so not part of the cached prefix.
  emit_memory
elif [ "$ROLE" = "reviewer" ]; then
  # Phase 1 (prompt-cache reclaim): the reviewer prefix is byte-stable across
  # issues. Only the curated reviewer-contract + the AGENTS.md quality-contract
  # section (the rules a reviewer actually enforces) live inside the cache
  # boundary; per-issue memory + scaffolding (and the PR diff, appended by
  # gen-reviewer-prompt.sh) go BELOW the closing marker.

  printf '<!-- CACHE BOUNDARY -->\n'

  # 1. Curated reviewer contract (NOT the ~14KB SKILL.md monitor-loop machinery).
  printf '## Reviewer contract\n\n'
  cat "$REVIEWER_CONTRACT"
  printf '\n'

  # 2. AGENTS.md ## Implementation-quality contract section — the enforcement
  #    narrative a reviewer applies (Enforcement, opt-out grammar, env-var
  #    contract). The RULE_ID table + Corrective directive map subsections are
  #    OMITTED here because reviewer-contract.md above already carries them
  #    verbatim: the table must appear EXACTLY once in the cached prefix (no
  #    duplicate re-extraction). validate.sh flags contract↔AGENTS.md drift.
  printf '## AGENTS.md — Implementation-quality contract (enforcement)\n\n'
  awk '
    /^## Implementation-quality contract/ { in_sec=1; print; next }
    /^## / && in_sec { exit }
    /^### RULE_ID table/ && in_sec { skip=1; next }
    /^### Corrective directive map/ && in_sec { skip=1; next }
    /^### / && skip { skip=0 }
    in_sec && !skip { print }
  ' "$AGENTS_MD"
  printf '\n'

  printf '<!-- CACHE BOUNDARY -->\n'

  # Below the boundary: per-issue, NOT part of the cached prefix.
  emit_memory
  emit_lockstep
  emit_scaffolding
 elif [ "$ROLE" = "decomposer" ]; then
  # M1 prefix slim: the decomposer prefix is byte-stable across specs. Only the
  # curated Phase 3 contract + the AGENTS.md issue-quality + small-LLM sections
  # live inside the cache boundary; per-issue memory + scaffolding go BELOW the
  # closing marker.
  printf '<!-- CACHE BOUNDARY -->\n'

  # 1. Curated decomposer contract (NOT the 67KB SKILL.md).
  printf '## Decomposer contract\n\n'
  cat "$DECOMPOSER_CONTRACT"
  printf '\n'

  # 2. AGENTS.md issue-quality + small-LLM sections — the contract the decomposer
  #    must satisfy when filing issues.
  printf '## AGENTS.md — issue-quality + small-LLM target\n\n'
  awk '
    /^(## Issue-quality contract|## Small-LLM target)/ { in_sec=1; print; next }
    /^## / && in_sec { in_sec=0 }
    in_sec { print }
  ' "$AGENTS_MD"
  printf '\n'

  printf '<!-- CACHE BOUNDARY -->\n'

  # Below the boundary: per-issue, NOT part of the cached prefix.
  emit_memory
  emit_lockstep
  emit_scaffolding
elif [ "$ROLE" = "classifier" ]; then
  # M1 prefix slim: the classifier prefix is byte-stable across issues. Only the
  # curated Phase 3.5 contract + the AGENTS.md subagent-model-selection section
  # live inside the cache boundary; per-issue memory + scaffolding go BELOW the
  # closing marker.
  printf '<!-- CACHE BOUNDARY -->\n'

  # 1. Curated classifier contract (NOT the ~18KB SKILL.md).
  printf '## Classifier contract\n\n'
  cat "$CLASSIFIER_CONTRACT"
  printf '\n'

  # 2. AGENTS.md subagent-model-selection section — the two-tier rules the
  #    classifier applies.
  printf '## AGENTS.md — subagent model selection\n\n'
  awk '
    /^## Subagent model selection/ { in_sec=1; print; next }
    /^## / && in_sec { in_sec=0 }
    in_sec { print }
  ' "$AGENTS_MD"
  printf '\n'

  printf '<!-- CACHE BOUNDARY -->\n'

  # Below the boundary: per-issue, NOT part of the cached prefix.
  emit_memory
  emit_lockstep
  emit_scaffolding
fi
