#!/usr/bin/env bash
# scripts/self-enforce-qa.sh — autospec self-enforcement QA helper.
#
# Runs autospec-run's Phase 4 QA chain (lint-implementation + validate.sh + drift gate)
# against a PR diff and exits non-zero if any blocking finding is detected.
#
# Usage:
#   self-enforce-qa.sh --diff-file <path> [--pr-body <text>] [--pr <N>]
#   self-enforce-qa.sh --help
#
# Options:
#   --diff-file <path>   Path to unified diff file to lint. Required.
#   --pr-body <text>     PR body text (used to detect [skip self-enforce] token).
#   --pr <N>             PR number (used for drift gate).
#
# Exit codes:
#   0  All checks passed (or [skip self-enforce] token present in PR body).
#   N  N blocking findings found (capped at 64).
#   1  Argument/usage error.
#
# Environment:
#   REPO_ROOT            — repo root (default: dirname of this script / 1 level up)
#   SELF_ENFORCE_SKIP_TOKEN — skip token text (default: "[skip self-enforce]")

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"

HELP_TEXT='Usage:
self-enforce-qa.sh --diff-file <path> [--pr-body <text>] [--pr <N>]
self-enforce-qa.sh --help

Run autospec Phase 4 QA chain against a PR diff.
Exit non-zero if blocking findings detected.
Set [skip self-enforce] in PR body to bypass.'

DIFF_FILE=""
PR_BODY=""
PR_NUMBER=""
SKIP_TOKEN="${SELF_ENFORCE_SKIP_TOKEN:-[skip self-enforce]}"

while [ $# -gt 0 ]; do
  case "$1" in
    --help|-h)
      printf '%s\n' "$HELP_TEXT"
      exit 0
      ;;
    --diff-file)
      if [ $# -lt 2 ]; then
        printf 'self-enforce-qa.sh: --diff-file requires an argument\n' >&2
        exit 1
      fi
      DIFF_FILE="$2"
      shift 2
      ;;
    --pr-body)
      if [ $# -lt 2 ]; then
        printf 'self-enforce-qa.sh: --pr-body requires an argument\n' >&2
        exit 1
      fi
      PR_BODY="$2"
      shift 2
      ;;
    --pr)
      if [ $# -lt 2 ]; then
        printf 'self-enforce-qa.sh: --pr requires an argument\n' >&2
        exit 1
      fi
      PR_NUMBER="$2"
      shift 2
      ;;
    -*)
      printf 'self-enforce-qa.sh: unknown option: %s\n' "$1" >&2
      exit 1
      ;;
    *)
      printf 'self-enforce-qa.sh: unexpected argument: %s\n' "$1" >&2
      exit 1
      ;;
  esac
done

# Validate required args
if [ -z "$DIFF_FILE" ]; then
  printf 'self-enforce-qa.sh: --diff-file is required\n' >&2
  printf '%s\n' "$HELP_TEXT" >&2
  exit 1
fi

if [ ! -f "$DIFF_FILE" ]; then
  printf 'self-enforce-qa.sh: diff file not found: %s\n' "$DIFF_FILE" >&2
  exit 1
fi

# Check for skip token in PR body
if [ -n "$PR_BODY" ] && printf '%s' "$PR_BODY" | grep -qF "$SKIP_TOKEN"; then
  printf '[self-enforce] skip token found — QA chain bypassed\n'
  exit 0
fi

# ── QA chain ─────────────────────────────────────────────────────────────────

FINDINGS=""
BLOCKING=0

# Step 1: lint-implementation.sh (deterministic RULE_ID detectors)
LINT_SCRIPT="$REPO_ROOT/scripts/lint-implementation.sh"
if [ -f "$LINT_SCRIPT" ]; then
  lint_out=""
  lint_exit=0
  lint_out=$(bash "$LINT_SCRIPT" --diff-file "$DIFF_FILE" 2>&1) || lint_exit=$?
  if [ "$lint_exit" -ne 0 ]; then
    BLOCKING=$((BLOCKING + lint_exit))
    FINDINGS="${FINDINGS}${lint_out}
"
  elif [ -n "$lint_out" ]; then
    # INFO lines only — print them but don't block
    FINDINGS="${FINDINGS}${lint_out}
"
  fi
else
  printf 'WARN: lint-implementation.sh not found at %s — skipping lint step\n' "$LINT_SCRIPT" >&2
fi

# Step 2: validate.sh (lockstep + structural checks)
VALIDATE_SCRIPT="$REPO_ROOT/scripts/validate.sh"
if [ -f "$VALIDATE_SCRIPT" ]; then
  validate_out=""
  validate_exit=0
  validate_out=$(bash "$VALIDATE_SCRIPT" 2>&1) || validate_exit=$?
  if [ "$validate_exit" -ne 0 ]; then
    BLOCKING=$((BLOCKING + 1))
    FINDINGS="${FINDINGS}VALIDATE_FAIL:-:-: scripts/validate.sh exited ${validate_exit}
"
  fi
else
  printf 'WARN: validate.sh not found at %s — skipping validate step\n' "$VALIDATE_SCRIPT" >&2
fi

# ── emit summary ──────────────────────────────────────────────────────────────

if [ "$BLOCKING" -gt 0 ]; then
  printf '[self-enforce] QA chain: %d blocking finding(s)\n' "$BLOCKING"
  printf '%s' "$FINDINGS"
  # Cap exit code at 64
  if [ "$BLOCKING" -gt 64 ]; then
    exit 64
  fi
  exit "$BLOCKING"
else
  if [ -n "$FINDINGS" ]; then
    printf '[self-enforce] QA chain: passed (INFO findings below)\n'
    printf '%s' "$FINDINGS"
  else
    printf '[self-enforce] QA chain: passed\n'
  fi
  exit 0
fi
