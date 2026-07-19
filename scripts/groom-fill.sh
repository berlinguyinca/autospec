#!/usr/bin/env bash
# scripts/groom-fill.sh — LLM template-fill for a needs-template issue.
#
# Shells out to `codex exec` (override via AUTOSPEC_GROOM_FILL_BIN) with the raw
# issue + the autospec template contract, then validates the result with
# groom-validate.sh (override via AUTOSPEC_GROOM_VALIDATE_BIN), retrying up to
# --attempts with the linter findings fed back as directives.
#
# Fail-closed: codex absent/error or validation never passing → ok:false; the
# caller MUST hold the issue for a human (never promote an unvalidated body).
#
# Usage:
#   groom-fill.sh --issue N --repo O/R [--attempts K] [--title T --body B]
# Output:
#   rc 0: {"ok":true,"body":"<filled template>"}
#   rc 1: {"ok":false,"reason":"codex-absent|codex-error|attempts-exhausted"}
#         (a body that never passes groom-validate falls through to attempts-exhausted)
set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
FILL_BIN="${AUTOSPEC_GROOM_FILL_BIN:-codex}"
VALIDATE_BIN="${AUTOSPEC_GROOM_VALIDATE_BIN:-$SCRIPT_DIR/groom-validate.sh}"
GH_BIN="${AUTOSPEC_GH_BIN:-gh}"

ISSUE="" REPO="" ATTEMPTS="" TITLE="" BODY="" HAVE_BODY=0
while [ $# -gt 0 ]; do
  case "$1" in
    --issue) ISSUE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --attempts) ATTEMPTS="${2:-}"; shift 2 ;;
    --title) TITLE="${2:-}"; shift 2 ;;
    --body) BODY="${2:-}"; HAVE_BODY=1; shift 2 ;;
    --help|-h) printf 'Usage: groom-fill.sh --issue N --repo O/R [--attempts K] [--title T --body B]\n'; exit 0 ;;
    *) printf 'groom-fill.sh: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done
[ -n "$ISSUE" ] || { printf 'groom-fill.sh: --issue required\n' >&2; exit 2; }
[ -n "$REPO" ] || { printf 'groom-fill.sh: --repo required\n' >&2; exit 2; }

fail() { jq -cn --arg r "$1" '{ok:false,reason:$r}'; exit 1; }

# codex binary must be resolvable (absolute path OR on PATH).
if ! command -v "$FILL_BIN" >/dev/null 2>&1 && [ ! -x "$FILL_BIN" ]; then
  fail "codex-absent"
fi

# Resolve attempts from arg → config → default 2.
if [ -z "$ATTEMPTS" ]; then
  if [ -f "$SCRIPT_DIR/../skills/autospec-shared/scripts/grooming-config.sh" ]; then
    ATTEMPTS="$(bash "$SCRIPT_DIR/../skills/autospec-shared/scripts/grooming-config.sh" \
                  --key budget.groom_attempts_per_issue 2>/dev/null || printf '2')"
  fi
fi
case "$ATTEMPTS" in ''|*[!0-9]*) ATTEMPTS=2 ;; esac

# Fetch title/body if not supplied.
if [ "$HAVE_BODY" -ne 1 ]; then
  raw="$("$GH_BIN" issue view "$ISSUE" --repo "$REPO" --json title,body 2>/dev/null || printf '')"
  if [ -n "$raw" ]; then
    TITLE="$(printf '%s' "$raw" | jq -r '.title // ""')"
    BODY="$(printf '%s' "$raw" | jq -r '.body // ""')"
  fi
fi

tmp_body="$(mktemp "${TMPDIR:-/tmp}/groom-fill.XXXXXX")"
trap 'rm -f "$tmp_body"' EXIT

directives=""
attempt=1
while [ "$attempt" -le "$ATTEMPTS" ]; do
  prompt="$(printf 'Fill the autospec issue template for the following raw issue.\nReturn ONLY the filled markdown body.\n\nTITLE: %s\n\nBODY:\n%s\n%s\n' \
            "$TITLE" "$BODY" "${directives:+PREVIOUS VALIDATION FINDINGS TO FIX:\n$directives}")"

  set +e
  # --skip-git-repo-check matches autospec's canonical codex invocation
  # (peer-review / advisor dispatcher). Without it `codex exec` refuses to run in
  # a non-trusted / non-git directory ("Not inside a trusted directory"), which
  # would fail-close every groom to hold on an otherwise-working codex install.
  filled="$(printf '%s' "$prompt" | "$FILL_BIN" exec --skip-git-repo-check 2>/dev/null)"
  rc=$?
  set -e
  if [ "$rc" -ne 0 ]; then
    fail "codex-error"
  fi
  if [ -z "$filled" ]; then
    directives="codex returned empty body"
    attempt=$((attempt + 1))
    continue
  fi

  printf '%s' "$filled" > "$tmp_body"
  set +e
  vout="$("$VALIDATE_BIN" "$tmp_body" 2>/dev/null)"
  vrc=$?
  set -e
  if [ "$vrc" -eq 0 ]; then
    jq -cn --arg b "$filled" '{ok:true,body:$b}'
    exit 0
  fi
  directives="$(printf '%s' "$vout" | jq -r '(.findings // []) | join("; ")' 2>/dev/null || printf '')"
  attempt=$((attempt + 1))
done

fail "attempts-exhausted"
