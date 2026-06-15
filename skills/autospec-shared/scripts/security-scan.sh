#!/usr/bin/env bash
# security-scan.sh — run deterministic security scanners and normalize every
# finding into the gap JSON contract (one compact JSON object per stdout line):
#   {gap_id, dimension, severity, file, line, title, body, dedupe_key}
#
# Usage:
#   security-scan.sh --tree [--root <dir>] [--only <classes>]
#   security-scan.sh --diff <base>          # scan only files changed vs <base>
#   security-scan.sh --help
#
# Dimensions: secrets | vuln | injection | license | pii | cve
# Severity:   must-fix | nice-to-have
#
# A missing scanner is NOT fatal: emit a loud WARN to stderr naming the gap and
# the affected dimension, then continue (the caller's LLM pass covers it).
# Fail-closed only if the engine itself cannot run (no jq) -> exit 2.
#
# Environment:
#   AUTOSPEC_SCRIPTS_DIR        — sibling scripts dir (default: script dir)
#   AUTOSPEC_SECSCAN_FORCE_LLM  — 1 = skip all scanners (LLM-only), default 0
#
# Exit codes:
#   0  ran (findings may or may not exist)
#   2  cannot run at all (jq missing) — caller should fail-closed/block
#
# Requires: bash 3.2+, jq

set +e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTOSPEC_SCRIPTS_DIR="${AUTOSPEC_SCRIPTS_DIR:-$SCRIPT_DIR}"
# shellcheck source=gap-json-lib.sh
. "$AUTOSPEC_SCRIPTS_DIR/gap-json-lib.sh"

command -v jq >/dev/null 2>&1 || { echo "security-scan: FATAL jq missing — fail-closed" >&2; exit 2; }

MODE="" ROOT="." BASE="" ONLY="" GAP_N=0
while [ $# -gt 0 ]; do
  case "$1" in
    --tree) MODE=tree ;;
    --diff) MODE=diff; shift; BASE="${1:-}" ;;
    --root) shift; ROOT="${1:-.}" ;;
    --only) shift; ONLY="${1:-}" ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "security-scan: unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done

# In --diff mode, narrow ROOT to a temp tree of only the changed files so
# tree-walking scanners (semgrep/license/trivy/gitleaks) see just the diff.
if [ "$MODE" = "diff" ]; then
  if ! command -v git >/dev/null 2>&1; then
    echo "security-scan: WARN git missing — cannot scope --diff, scanning full tree" >&2
  else
    if [ -z "$BASE" ] || ! git -C "$ROOT" rev-parse --verify "$BASE" >/dev/null 2>&1; then
      echo "security-scan: WARN --diff base '$BASE' is empty or not a valid ref; results may be incomplete" >&2
    fi
    _scan_tmp="$(mktemp -d)"
    trap 'rm -rf "$_scan_tmp"' EXIT
    { git -C "$ROOT" diff --name-only "$BASE" 2>/dev/null; \
      git -C "$ROOT" ls-files --others --exclude-standard 2>/dev/null; } \
      | sort -u | while IFS= read -r rel; do
        [ -n "$rel" ] || continue
        [ -f "$ROOT/$rel" ] || continue
        mkdir -p "$_scan_tmp/$(dirname "$rel")"
        cp "$ROOT/$rel" "$_scan_tmp/$rel"
      done
    ROOT="$_scan_tmp"
  fi
fi

# want <class> — is this dimension in scope? (empty --only = all)
want() { [ -z "$ONLY" ] && return 0; case " $ONLY " in *" $1 "*) return 0;; esac; return 1; }

# emit_gap <dimension> <severity> <file> <line> <title> <body>
emit_gap() {
  GAP_N=$((GAP_N + 1))
  local dim="$1" sev="$2" file="$3" line="$4" title="$5" body="$6"
  local ddk; ddk="$(gap_title_hash "$dim:$file:$title")"
  jq -nc --arg id "G$GAP_N" --arg dim "$dim" --arg sev "$sev" \
        --arg file "$file" --argjson line "${line:-0}" --arg title "$title" \
        --arg body "$body" --arg ddk "$ddk" \
        '{gap_id:$id,dimension:$dim,severity:$sev,file:$file,line:$line,title:$title,body:$body,dedupe_key:$ddk}'
}

# warn_missing <tool> <dimension>
warn_missing() {
  echo "security-scan: WARN scanner '$1' missing — '$2' falls back to LLM-only" >&2
}

# ── secrets: gitleaks ────────────────────────────────────────────────────────
scan_secrets() {
  want secrets || return 0
  if [ "${AUTOSPEC_SECSCAN_FORCE_LLM:-0}" = "1" ] || ! command -v gitleaks >/dev/null 2>&1; then
    warn_missing gitleaks secrets; return 0
  fi
  local report; report="$(mktemp)"
  if gitleaks dir --help >/dev/null 2>&1; then
    gitleaks dir --no-banner --report-format json --report-path "$report" "$ROOT" >/dev/null 2>&1
  else
    gitleaks detect --no-git --no-banner --source "$ROOT" \
      --report-format json --report-path "$report" >/dev/null 2>&1
  fi
  [ -s "$report" ] || { rm -f "$report"; return 0; }
  jq -c '.[]?' "$report" 2>/dev/null | while IFS= read -r f; do
    local file line title
    file="$(printf '%s' "$f" | jq -r '.File // ""')"
    file="${file#$ROOT/}"
    line="$(printf '%s' "$f" | jq -r '.StartLine // 0')"
    title="$(printf '%s' "$f" | jq -r '.Description // .RuleID // "secret"')"
    emit_gap secrets must-fix "$file" "$line" "$title" \
      "Hardcoded secret detected by gitleaks. Remove from code AND rotate the credential — a committed secret is compromised."
  done
  rm -f "$report"
}

# ── vuln/injection: semgrep ──────────────────────────────────────────────────
scan_semgrep() {
  want vuln || want injection || return 0
  if [ "${AUTOSPEC_SECSCAN_FORCE_LLM:-0}" = "1" ] || ! command -v semgrep >/dev/null 2>&1; then
    warn_missing semgrep vuln; return 0
  fi
  local out; out="$(semgrep --config=auto --json --quiet "$ROOT" 2>/dev/null)"
  [ -n "$out" ] || return 0
  printf '%s' "$out" | jq -c '.results[]?' 2>/dev/null | while IFS= read -r r; do
    local file line msg sev gsev cid dim
    file="$(printf '%s' "$r" | jq -r '.path // ""')"
    file="${file#$ROOT/}"
    line="$(printf '%s' "$r" | jq -r '.start.line // 0')"
    msg="$(printf '%s' "$r"  | jq -r '.extra.message // .check_id // "vulnerability"')"
    sev="$(printf '%s' "$r"  | jq -r '.extra.severity // "WARNING"')"
    cid="$(printf '%s' "$r"  | jq -r '.check_id // ""')"
    [ "$sev" = "ERROR" ] && gsev="must-fix" || gsev="nice-to-have"
    case "$cid" in
      *sql*|*sqli*|*injection*|*xss*|*ssrf*|*command-injection*|*tainted*) dim=injection ;;
      *) dim=vuln ;;
    esac
    # Respect --only after we know the real dimension.
    if [ -n "$ONLY" ]; then case " $ONLY " in *" $dim "*) : ;; *) continue ;; esac; fi
    emit_gap "$dim" "$gsev" "$file" "$line" "$msg" \
      "semgrep flagged a security pattern ($cid). Validate input at the boundary; never eval/exec untrusted data; parameterize SQL."
  done
}

# ── license/IP: license-checker (deps) + copyleft header heuristic (source) ───
scan_license() {
  want license || return 0
  # Copyleft header heuristic over source tree (advisory-fix, human confirms).
  if command -v grep >/dev/null 2>&1; then
    grep -rliE 'GNU (General|Lesser) Public License|GPL|AGPL' "$ROOT" 2>/dev/null \
      | while IFS= read -r file; do
        emit_gap license must-fix "$file" 0 "Possible copyleft (GPL/AGPL) license header" \
          "Copyleft license text found. Confirm this code can ship under the project license — human review required before merge."
      done
  fi
  command -v license-checker >/dev/null 2>&1 || { warn_missing license-checker license; return 0; }
  # Dependency license scan runs only where a package.json exists.
  [ -f "$ROOT/package.json" ] || return 0
  ( cd "$ROOT" && license-checker --json --production 2>/dev/null ) \
    | jq -rc 'to_entries[]? | select(.value.licenses | test("GPL|AGPL"; "i")) | {pkg:.key, lic:.value.licenses}' 2>/dev/null \
    | while IFS= read -r e; do
        emit_gap license must-fix "package.json" 0 \
          "Copyleft dependency: $(printf '%s' "$e" | jq -r '.pkg')" \
          "Dependency license $(printf '%s' "$e" | jq -r '.lic') may be incompatible with the project license."
      done
}

# ── CVE: trivy (advisory — only critical+fixable should ever block) ──────────
scan_trivy() {
  want cve || return 0
  command -v trivy >/dev/null 2>&1 || { warn_missing trivy cve; return 0; }
  local out; out="$(trivy fs --quiet --format json "$ROOT" 2>/dev/null)"
  [ -n "$out" ] || return 0
  printf '%s' "$out" | jq -c '.Results[]?.Vulnerabilities[]?' 2>/dev/null | while IFS= read -r v; do
    local id pkg sev fixed gsev
    id="$(printf '%s' "$v"    | jq -r '.VulnerabilityID')"
    pkg="$(printf '%s' "$v"   | jq -r '.PkgName')"
    sev="$(printf '%s' "$v"   | jq -r '.Severity')"
    fixed="$(printf '%s' "$v" | jq -r '.FixedVersion // ""')"
    if [ "$sev" = "CRITICAL" ] && [ -n "$fixed" ]; then gsev="must-fix"; else gsev="nice-to-have"; fi
    emit_gap cve "$gsev" "package-lock.json" 0 "$id in $pkg ($sev)" \
      "$(printf '%s' "$v" | jq -r '.Title // ""'). Fixed in: ${fixed:-none available}."
  done
}

# ── PII in logs/output: deterministic heuristic (LLM triage refines) ─────────
# Flags personal data emitted through a log/print sink AS A VALUE (adjacent to
# =/: or via .member / ["index"] access) — not bare English words. First pass;
# the skill's LLM triage confirms and may add cases this misses.
scan_pii() {
  want pii || return 0
  command -v grep >/dev/null 2>&1 || { warn_missing grep pii; return 0; }
  local sinks='console\.(log|info|warn|error|debug)|System\.out|fmt\.Print|logger?\.|logging\.|printf|println|print\(|[[:space:]]echo[[:space:]]'
  local pii='ssn|social.?security|credit.?card|card.?number|cardnumber|dob|date.?of.?birth|email|phone|passport|national.?id'
  # PII must look like a value: name= / name: / .name / ["name"] / ['name']
  local val="((${pii})[[:space:]]*[=:]|[.\\[][\"']?(${pii}))"
  grep -rInEi "(${sinks}).*${val}" "$ROOT" 2>/dev/null | while IFS= read -r hit; do
    local file line
    file="$(printf '%s' "$hit" | cut -d: -f1)"; file="${file#$ROOT/}"
    line="$(printf '%s' "$hit" | cut -d: -f2)"
    emit_gap pii must-fix "$file" "$line" "Possible PII written to a log/output sink" \
      "Personal data (SSN/email/phone/card/DOB) appears to be logged or printed. Redact or omit PII from logs; emit a stable non-identifying token instead."
  done
}

# ── Prompt injection: untrusted input concatenated into an LLM/prompt sink ────
# Advisory candidate (nice-to-have): a line referencing a prompt/LLM sink that
# also performs raw string concatenation. Regex cannot prove taint flow, so the
# skill's LLM triage upgrades confirmed cases to must-fix. Conservative to limit
# false positives.
scan_promptinj() {
  want injection || return 0
  command -v grep >/dev/null 2>&1 || { warn_missing grep injection; return 0; }
  local sinks='prompt|messages|completion|chatcompletion|\.chat\.|\.complete|\.generate|openai|anthropic|claude|llm'
  # raw concatenation: "literal" + var   OR   var + "literal"   OR f"...{x}..."
  local concat='("[^"]*"[[:space:]]*\+|\+[[:space:]]*"[^"]*"|f"[^"]*\{)'
  grep -rInEi "(${sinks})" "$ROOT" 2>/dev/null | grep -EI "${concat}" 2>/dev/null | while IFS= read -r hit; do
    local file line
    file="$(printf '%s' "$hit" | cut -d: -f1)"; file="${file#$ROOT/}"
    line="$(printf '%s' "$hit" | cut -d: -f2)"
    emit_gap injection nice-to-have "$file" "$line" "Possible prompt injection: input concatenated into an LLM/prompt" \
      "Untrusted input appears to be concatenated into a prompt/LLM call. Separate instructions from data, validate/escape user input, and prefer structured message fields over string concatenation. (LLM triage confirms severity.)"
  done
}

scan_secrets
scan_semgrep
scan_license
scan_trivy
scan_pii
scan_promptinj
exit 0
