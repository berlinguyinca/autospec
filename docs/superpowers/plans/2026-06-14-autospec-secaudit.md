# autospec-secaudit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a security / IP / secret-leak guard to autospec with two entry points sharing one detection engine: a blocking Phase 4 per-PR gate (auto-fix + re-scan loop) and a standalone `/autospec-secaudit` skill (repo-wide sweep, auto-fires post-batch).

**Architecture:** Deterministic scanners (gitleaks, semgrep, trivy, license-checker) run first and normalize every finding into the existing gap JSON schema; an LLM triage pass confirms/augments. Scanners auto-install best-effort via the extended `ensure-tool.sh`. One shared engine (`security-scan.sh` + `security-remediation-loop.sh`) is called both by the standalone skill and by Phase 4.

**Tech Stack:** POSIX/bash 3.2 shell scripts, `jq`, `bats` tests, the autospec trio-skill convention (SKILL.md + codex/prompt.md + opencode/agent.md), `gap-json-lib.sh` / `emit-gaps.sh` contract reuse.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `skills/autospec-shared/scripts/ensure-tool.sh` (modify) | Add `gitleaks`, `semgrep`, `trivy`, `license-checker` to baked-in installer table. |
| `skills/autospec-shared/scripts/security-scan.sh` (create) | Run scanners over `--diff <base>` or `--tree`; normalize → gap JSON array; loud WARN + LLM-fallback marker per missing scanner; fail-closed only if it cannot run at all. |
| `skills/autospec-shared/scripts/security-remediation-loop.sh` (create) | scan → (caller LLM-triages/fixes) → re-scan loop, capped `AUTOSPEC_SEC_MAX_ROUNDS` (default 3); block decision; secret-rotation annotation. |
| `skills/autospec-shared/tests/unit/security-scan.bats` (create) | Scanner mapping + scanner-absent fallback tests. |
| `skills/autospec-shared/tests/unit/security-remediation-loop.bats` (create) | Loop cap, block-on-survivor, secret-redaction path. |
| `skills/autospec-shared/tests/fixtures/secaudit/*` (create) | Raw planted-finding fixtures (AWS key, SQLi, eval, GPL header, PII log). |
| `skills/autospec-secaudit/SKILL.md` (create) | Standalone skill body (canonical). |
| `skills/autospec-secaudit/codex/prompt.md` (create) | Byte-identical to SKILL.md body (lock-step). |
| `skills/autospec-secaudit/opencode/agent.md` (create) | Frontmatter + identical body (lock-step). |
| `skills/autospec-secaudit/{install.sh,uninstall.sh,README.md}` (create) | Mirror sibling skill scaffolding. |
| `skills/autospec-run/prompts/phase4-implementer.md` (modify) | Wire the gate + remediation loop into Phase 4 before merge. |
| `skills/autospec-run/prompts/implementer-contract.md` (modify) | Extend the `SECURITY` directive row. |
| `autospec validate` (modify) | Add named-content checks for the new trio. |

**Lock-step note:** `validate.sh check_lockstep()` byte-diffs `SKILL.md` body against `codex/prompt.md` and against `opencode/agent.md` body. Every trio file must also carry a `## Stop mode` heading (enforced by validate.sh). The `codex/prompt.md` file is the SKILL.md body with a **leading blank line** — preserve it.

---

## Task 1: Extend ensure-tool.sh with the scanner table entries

**Files:**
- Modify: `skills/autospec-shared/scripts/ensure-tool.sh:172-248` (the `case "$TOOL"` block) and the doc header tool list at `:15-16`.

- [ ] **Step 1: Write the failing test**

Create `skills/autospec-shared/tests/unit/ensure-tool-scanners.bats`:

```bash
#!/usr/bin/env bats
# ensure-tool-scanners.bats — the security scanners are present in the baked-in table.

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    ENSURE="${SCRIPT_DIR}/scripts/ensure-tool.sh"
}

@test "gitleaks is a known tool (not the unknown no-op path)" {
    run grep -E '^\s+gitleaks\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "semgrep is a known tool" {
    run grep -E '^\s+semgrep\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "trivy is a known tool" {
    run grep -E '^\s+trivy\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "license-checker is a known tool" {
    run grep -E '^\s+license-checker\)' "$ENSURE"
    [ "$status" -eq 0 ]
}

@test "already-present scanner is a no-op exit 0" {
    # 'jq' is already installed in CI; ensure-tool must short-circuit.
    run bash "$ENSURE" jq
    [ "$status" -eq 0 ]
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/ensure-tool-scanners.bats`
Expected: FAIL — the four `grep` tests fail (no table entries yet).

- [ ] **Step 3: Add the four case entries**

In `ensure-tool.sh`, insert these cases inside the `case "$TOOL" in` block (alphabetically, before `*)`). gitleaks/trivy ship Homebrew + binary releases; semgrep is pip/pipx; license-checker is npm:

```bash
  gitleaks)
    _try_brew gitleaks || _try_winget gitleaks.gitleaks || _try_choco gitleaks || _try_scoop gitleaks || true
    ;;
  semgrep)
    # Python tool: pipx (isolated venv) → uv → pip --user → brew.
    _try_pipx semgrep || _try_uv semgrep || _try_pip semgrep || _try_brew semgrep || true
    ;;
  trivy)
    _try_brew trivy || _try_winget AquaSecurity.Trivy || _try_choco trivy || _try_scoop trivy || true
    ;;
  license-checker)
    _try_npm license-checker || true
    ;;
```

Also update the doc-header supported-tools list (`:15-16`) to include `gitleaks, license-checker, semgrep, trivy`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bats skills/autospec-shared/tests/unit/ensure-tool-scanners.bats`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/ensure-tool.sh skills/autospec-shared/tests/unit/ensure-tool-scanners.bats
git commit -m "feat(secaudit): add security scanners to ensure-tool installer table"
```

---

## Task 2: Fixtures with raw planted findings

**Files:**
- Create: `skills/autospec-shared/tests/fixtures/secaudit/aws-key.diff`
- Create: `skills/autospec-shared/tests/fixtures/secaudit/sqli.py`
- Create: `skills/autospec-shared/tests/fixtures/secaudit/eval.py`
- Create: `skills/autospec-shared/tests/fixtures/secaudit/gpl-header.c`
- Create: `skills/autospec-shared/tests/fixtures/secaudit/pii-log.js`

> **Why raw strings, not derived:** per the self-consistent-fixture rule, fixtures must NOT be built from the SUT's own derivation expression — plant real-world-shaped strings so a bug in detection actually fails a test.

- [ ] **Step 1: Create the secret fixture**

`aws-key.diff` (a unified diff so `security-scan.sh --diff` can be exercised; the key is a canonical test pattern, not a live credential):

```diff
diff --git a/config.py b/config.py
new file mode 100644
--- /dev/null
+++ b/config.py
@@ -0,0 +1,2 @@
+AWS_ACCESS_KEY_ID = "AKIAIOSFODNN7EXAMPLE"
+AWS_SECRET_ACCESS_KEY = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
```

- [ ] **Step 2: Create the vuln fixtures**

`sqli.py`:

```python
def get_user(cursor, name):
    cursor.execute("SELECT * FROM users WHERE name = '" + name + "'")
    return cursor.fetchone()
```

`eval.py`:

```python
def run(user_input):
    return eval(user_input)
```

`gpl-header.c`:

```c
/* This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License. */
int main(void) { return 0; }
```

`pii-log.js`:

```javascript
function login(user) {
  console.log("login attempt ssn=" + user.ssn + " email=" + user.email);
}
```

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-shared/tests/fixtures/secaudit/
git commit -m "test(secaudit): add raw planted-finding fixtures"
```

---

## Task 3: security-scan.sh — secret detection path (gitleaks → gap JSON)

**Files:**
- Create: `skills/autospec-shared/scripts/security-scan.sh`
- Test: `skills/autospec-shared/tests/unit/security-scan.bats`

- [ ] **Step 1: Write the failing test**

Create `security-scan.bats`:

```bash
#!/usr/bin/env bats
# security-scan.bats — security-scan.sh maps findings to the gap contract.

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    SCAN="${SCRIPT_DIR}/scripts/security-scan.sh"
    FIX="${SCRIPT_DIR}/tests/fixtures/secaudit"
    GAPLIB="${SCRIPT_DIR}/scripts/gap-json-lib.sh"
    TMP="$(mktemp -d /tmp/autospec-secscan-XXXXXX)"
    # Force LLM-only fallback OFF and scanners ON only where we stub them.
    export AUTOSPEC_SECSCAN_FORCE_LLM=0
}

teardown() { rm -rf "$TMP"; }

# Stub gitleaks on PATH that emits one finding for our fixture.
stub_gitleaks() {
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/gitleaks" <<'EOF'
#!/usr/bin/env bash
# Minimal stub: emit gitleaks JSON report to the --report-path arg.
for a in "$@"; do case "$prev" in --report-path) out="$a";; esac; prev="$a"; done
cat > "$out" <<'JSON'
[{"RuleID":"aws-access-key","File":"config.py","StartLine":1,"Description":"AWS Access Key","Secret":"AKIAIOSFODNN7EXAMPLE"}]
JSON
exit 1
EOF
    chmod +x "$TMP/bin/gitleaks"
    export PATH="$TMP/bin:$PATH"
}

@test "gitleaks finding becomes a valid secrets gap object" {
    stub_gitleaks
    run bash "$SCAN" --tree --root "$FIX" --only secrets
    [ "$status" -eq 0 ]
    # Each line of output is a gap object; first must validate + be dimension=secrets.
    first="$(printf '%s' "$output" | head -1)"
    run bash "$GAPLIB" --validate-file <(printf '%s' "$first")
    [ "$status" -eq 0 ]
    printf '%s' "$first" | jq -e '.dimension == "secrets" and .severity == "must-fix"'
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/security-scan.bats`
Expected: FAIL — `security-scan.sh` does not exist.

- [ ] **Step 3: Write the script (secret path + skeleton)**

Create `security-scan.sh`:

```bash
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
# Fail-closed only if the engine itself cannot run (no jq) → exit 2.
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
  gitleaks detect --no-banner --source "$ROOT" --report-format json --report-path "$report" >/dev/null 2>&1
  [ -s "$report" ] || { rm -f "$report"; return 0; }
  jq -c '.[]?' "$report" 2>/dev/null | while IFS= read -r f; do
    local file line title
    file="$(printf '%s' "$f" | jq -r '.File // ""')"
    line="$(printf '%s' "$f" | jq -r '.StartLine // 0')"
    title="$(printf '%s' "$f" | jq -r '.Description // .RuleID // "secret"')"
    emit_gap secrets must-fix "$file" "$line" "$title" \
      "Hardcoded secret detected by gitleaks. Remove from code AND rotate the credential — a committed secret is compromised."
  done
  rm -f "$report"
}

scan_secrets
# (vuln / injection / license / pii / cve added in later tasks)
exit 0
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats skills/autospec-shared/tests/unit/security-scan.bats`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/security-scan.sh skills/autospec-shared/tests/unit/security-scan.bats
git commit -m "feat(secaudit): security-scan.sh secrets path (gitleaks → gap JSON)"
```

---

## Task 4: security-scan.sh — vuln/injection (semgrep) + license + scanner-absent fallback

**Files:**
- Modify: `skills/autospec-shared/scripts/security-scan.sh` (add `scan_semgrep`, `scan_license`, call them)
- Modify: `skills/autospec-shared/tests/unit/security-scan.bats`

- [ ] **Step 1: Write the failing tests**

Append to `security-scan.bats`:

```bash
stub_semgrep() {
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/semgrep" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"results":[
 {"check_id":"python.lang.security.audit.eval-detected","path":"eval.py","start":{"line":2},"extra":{"severity":"ERROR","message":"eval() on user input"}},
 {"check_id":"python.sqlalchemy.security.sqli","path":"sqli.py","start":{"line":2},"extra":{"severity":"ERROR","message":"SQL injection"}}
]}
JSON
exit 0
EOF
    chmod +x "$TMP/bin/semgrep"
    export PATH="$TMP/bin:$PATH"
}

@test "semgrep ERROR findings map to vuln/injection must-fix gaps" {
    stub_semgrep
    run bash "$SCAN" --tree --root "$FIX" --only vuln
    [ "$status" -eq 0 ]
    # Two findings expected.
    [ "$(printf '%s\n' "$output" | grep -c '"dimension":"vuln"')" -eq 2 ]
    printf '%s' "$output" | grep -q '"severity":"must-fix"'
}

@test "missing scanner warns loudly and still exits 0 (LLM fallback)" {
    # No stubs on PATH; force a clean PATH without scanners.
    run env PATH="/usr/bin:/bin" bash "$SCAN" --tree --root "$FIX" --only secrets
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q "WARN scanner 'gitleaks' missing"
}

@test "engine fails closed (exit 2) when jq is unavailable" {
    run env PATH="$TMP/empty" bash "$SCAN" --tree --root "$FIX"
    [ "$status" -eq 2 ]
}
```

Add to `setup()`: `mkdir -p "$TMP/empty"`.

- [ ] **Step 2: Run test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/security-scan.bats`
Expected: FAIL — semgrep test finds 0 `vuln` gaps (no `scan_semgrep` yet).

- [ ] **Step 3: Add scanner functions**

In `security-scan.sh`, before the `scan_secrets` call line, add:

```bash
# ── vuln/injection: semgrep ──────────────────────────────────────────────────
scan_semgrep() {
  want vuln || want injection || return 0
  if [ "${AUTOSPEC_SECSCAN_FORCE_LLM:-0}" = "1" ] || ! command -v semgrep >/dev/null 2>&1; then
    warn_missing semgrep vuln; return 0
  fi
  local out; out="$(semgrep --config=auto --json --quiet "$ROOT" 2>/dev/null)"
  [ -n "$out" ] || return 0
  printf '%s' "$out" | jq -c '.results[]?' 2>/dev/null | while IFS= read -r r; do
    local file line msg sev gsev
    file="$(printf '%s' "$r" | jq -r '.path // ""')"
    line="$(printf '%s' "$r" | jq -r '.start.line // 0')"
    msg="$(printf '%s' "$r"  | jq -r '.extra.message // .check_id // "vulnerability"')"
    sev="$(printf '%s' "$r"  | jq -r '.extra.severity // "WARNING"')"
    [ "$sev" = "ERROR" ] && gsev="must-fix" || gsev="nice-to-have"
    emit_gap vuln "$gsev" "$file" "$line" "$msg" \
      "semgrep flagged a security pattern ($(printf '%s' "$r" | jq -r '.check_id')). Validate input at the boundary; never eval/exec untrusted data; parameterize SQL."
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
```

Then replace the bottom call section with:

```bash
scan_secrets
scan_semgrep
scan_license
# (pii / cve covered by the LLM triage pass + trivy in a follow-up)
exit 0
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats skills/autospec-shared/tests/unit/security-scan.bats`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/security-scan.sh skills/autospec-shared/tests/unit/security-scan.bats
git commit -m "feat(secaudit): semgrep + license scan paths and fail-closed/fallback behavior"
```

---

## Task 5: security-scan.sh — trivy (CVE, advisory) + diff-mode file filter

**Files:**
- Modify: `skills/autospec-shared/scripts/security-scan.sh`
- Modify: `skills/autospec-shared/tests/unit/security-scan.bats`

- [ ] **Step 1: Write the failing tests**

Append to `security-scan.bats`:

```bash
@test "trivy CVE findings are advisory (nice-to-have), not blocking" {
    mkdir -p "$TMP/bin"
    cat > "$TMP/bin/trivy" <<'EOF'
#!/usr/bin/env bash
cat <<'JSON'
{"Results":[{"Target":"package-lock.json","Vulnerabilities":[{"VulnerabilityID":"CVE-2021-1234","PkgName":"lodash","Severity":"HIGH","Title":"proto pollution","FixedVersion":""}]}]}
JSON
exit 0
EOF
    chmod +x "$TMP/bin/trivy"
    run env PATH="$TMP/bin:/usr/bin:/bin" bash "$SCAN" --tree --root "$FIX" --only cve
    [ "$status" -eq 0 ]
    printf '%s' "$output" | jq -e 'select(.dimension=="cve") | .severity == "nice-to-have"'
}

@test "--diff scopes findings to changed files only" {
    # init a tiny repo with one committed clean file + one staged secret file
    git -C "$TMP" init -q
    printf 'ok = 1\n' > "$TMP/clean.py"
    git -C "$TMP" add clean.py && git -C "$TMP" -c user.email=t@t -c user.name=t commit -qm base
    cp "$FIX/sqli.py" "$TMP/sqli.py"
    run bash "$SCAN" --diff HEAD --root "$TMP" --only vuln
    [ "$status" -eq 0 ]
    # clean.py must never appear; only changed file(s) scanned.
    ! printf '%s' "$output" | grep -q 'clean.py'
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/security-scan.bats`
Expected: FAIL — no `scan_trivy`, and `--diff` not yet filtering.

- [ ] **Step 3: Add trivy + diff scoping**

Add `scan_trivy` to `security-scan.sh`:

```bash
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
    # Advisory by default; only CRITICAL *with* a fix is must-fix.
    if [ "$sev" = "CRITICAL" ] && [ -n "$fixed" ]; then gsev="must-fix"; else gsev="nice-to-have"; fi
    emit_gap cve "$gsev" "package-lock.json" 0 "$id in $pkg ($sev)" \
      "$(printf '%s' "$v" | jq -r '.Title // ""'). Fixed in: ${fixed:-none available}."
  done
}
```

Add diff scoping near the top (after arg parse): when `MODE=diff`, compute the changed-file set and restrict scanners that walk the tree. The simplest correct approach — copy changed files into a temp scan root:

```bash
# In --diff mode, narrow ROOT to a temp tree of only the changed files so
# tree-walking scanners (semgrep/license/trivy/gitleaks) see just the diff.
if [ "$MODE" = "diff" ]; then
  command -v git >/dev/null 2>&1 || { echo "security-scan: WARN git missing — cannot scope --diff, scanning full tree" >&2; }
  if command -v git >/dev/null 2>&1; then
    _scan_tmp="$(mktemp -d)"
    git -C "$ROOT" diff --name-only "$BASE" 2>/dev/null | while IFS= read -r rel; do
      [ -f "$ROOT/$rel" ] || continue
      mkdir -p "$_scan_tmp/$(dirname "$rel")"
      cp "$ROOT/$rel" "$_scan_tmp/$rel"
    done
    ROOT="$_scan_tmp"
  fi
fi
```

Add `scan_trivy` to the call list before `exit 0`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bats skills/autospec-shared/tests/unit/security-scan.bats`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/security-scan.sh skills/autospec-shared/tests/unit/security-scan.bats
git commit -m "feat(secaudit): trivy CVE (advisory) + --diff file scoping"
```

---

## Task 6: security-remediation-loop.sh — block decision + round cap + secret rotation flag

**Files:**
- Create: `skills/autospec-shared/scripts/security-remediation-loop.sh`
- Test: `skills/autospec-shared/tests/unit/security-remediation-loop.bats`

> This script orchestrates the scan and the *block/continue* decision. The actual LLM-driven code fix between rounds is performed by the **caller** (Phase 4 implementer or the skill), which re-invokes this script. The loop owns: invoking `security-scan.sh`, deciding block vs pass, enforcing the round cap, and emitting the rotation annotation for any surviving secret.

- [ ] **Step 1: Write the failing test**

Create `security-remediation-loop.bats`:

```bash
#!/usr/bin/env bats
# security-remediation-loop.bats

setup() {
    SCRIPT_DIR="$(cd "$(dirname "${BATS_TEST_FILENAME}")/../.." && pwd)"
    LOOP="${SCRIPT_DIR}/scripts/security-remediation-loop.sh"
    TMP="$(mktemp -d /tmp/autospec-secloop-XXXXXX)"
    # Stub security-scan.sh via AUTOSPEC_SECSCAN_BIN so we control findings.
    export AUTOSPEC_STATE_DIR="$TMP/state"; mkdir -p "$AUTOSPEC_STATE_DIR"
}
teardown() { rm -rf "$TMP"; }

mk_scan_stub() {  # mk_scan_stub <file-with-findings-per-line>
    cat > "$TMP/scan.sh" <<EOF
#!/usr/bin/env bash
cat "$1"
exit 0
EOF
    chmod +x "$TMP/scan.sh"
    export AUTOSPEC_SECSCAN_BIN="$TMP/scan.sh"
}

@test "clean scan → decision=pass exit 0" {
    : > "$TMP/empty.txt"; mk_scan_stub "$TMP/empty.txt"
    run bash "$LOOP" --decide
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q 'decision=pass'
}

@test "surviving must-fix → decision=block exit 1" {
    printf '%s\n' '{"gap_id":"G1","dimension":"vuln","severity":"must-fix","file":"a.py","line":2,"title":"sqli","body":"x","dedupe_key":"k"}' > "$TMP/f.txt"
    mk_scan_stub "$TMP/f.txt"
    run bash "$LOOP" --decide
    [ "$status" -eq 1 ]
    printf '%s' "$output" | grep -q 'decision=block'
}

@test "must-fix secret emits a rotation annotation" {
    printf '%s\n' '{"gap_id":"G1","dimension":"secrets","severity":"must-fix","file":"c.py","line":1,"title":"AWS key","body":"x","dedupe_key":"k"}' > "$TMP/f.txt"
    mk_scan_stub "$TMP/f.txt"
    run bash "$LOOP" --decide
    printf '%s' "$output" | grep -qi 'ROTATE'
}

@test "advisory-only findings (nice-to-have) → decision=pass" {
    printf '%s\n' '{"gap_id":"G1","dimension":"cve","severity":"nice-to-have","file":"p","line":0,"title":"cve","body":"x","dedupe_key":"k"}' > "$TMP/f.txt"
    mk_scan_stub "$TMP/f.txt"
    run bash "$LOOP" --decide
    [ "$status" -eq 0 ]
    printf '%s' "$output" | grep -q 'decision=pass'
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bats skills/autospec-shared/tests/unit/security-remediation-loop.bats`
Expected: FAIL — script does not exist.

- [ ] **Step 3: Write the script**

Create `security-remediation-loop.sh`:

```bash
#!/usr/bin/env bash
# security-remediation-loop.sh — run security-scan.sh and decide block vs pass.
#
# The caller drives the actual code fix between rounds; this script owns the
# scan invocation, the block/pass decision, the round cap, and the secret
# rotation annotation. One round per invocation when called with --decide; the
# caller loops: fix → re-invoke until pass or AUTOSPEC_SEC_MAX_ROUNDS.
#
# Usage:
#   security-remediation-loop.sh --decide [--diff <base>] [--root <dir>]
#
# Block rule: decision=block (exit 1) iff any finding has severity==must-fix.
#             Otherwise decision=pass (exit 0). nice-to-have never blocks.
# Secret rule: every surviving must-fix secrets gap also prints a
#             "ROTATE: <file> — <title>" line to stdout for the PR body.
#
# Environment:
#   AUTOSPEC_SECSCAN_BIN    — path to security-scan.sh (default: sibling)
#   AUTOSPEC_SEC_MAX_ROUNDS — informational cap echoed for the caller (default 3)
#   AUTOSPEC_STATE_DIR      — state dir (default: ~/.autospec)
#
# Exit codes:
#   0  decision=pass
#   1  decision=block (must-fix survivors)
#   2  scan engine failed closed (could not run)
#
# Requires: bash 3.2+, jq

set +e
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCAN_BIN="${AUTOSPEC_SECSCAN_BIN:-$SCRIPT_DIR/security-scan.sh}"
MAX_ROUNDS="${AUTOSPEC_SEC_MAX_ROUNDS:-3}"

DIFF="" ROOT="." DECIDE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --decide) DECIDE=1 ;;
    --diff) shift; DIFF="${1:-}" ;;
    --root) shift; ROOT="${1:-.}" ;;
    -h|--help) grep '^#' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "remediation-loop: unknown arg $1" >&2; exit 2 ;;
  esac
  shift
done

# Build scan args.
set --
[ -n "$DIFF" ] && set -- --diff "$DIFF" || set -- --tree
set -- "$@" --root "$ROOT"

findings="$("$SCAN_BIN" "$@" 2>/dev/null)"; rc=$?
[ "$rc" -eq 2 ] && { echo "decision=block reason=engine-failed-closed"; exit 2; }

mustfix="$(printf '%s\n' "$findings" | jq -rc 'select(.severity=="must-fix")' 2>/dev/null)"

# Secret rotation annotations.
printf '%s\n' "$mustfix" | jq -r 'select(.dimension=="secrets") | "ROTATE: \(.file) — \(.title)"' 2>/dev/null

if [ -n "$mustfix" ] && [ "$mustfix" != "" ]; then
  count="$(printf '%s\n' "$mustfix" | grep -c '{')"
  echo "decision=block must_fix=$count max_rounds=$MAX_ROUNDS"
  exit 1
fi
echo "decision=pass max_rounds=$MAX_ROUNDS"
exit 0
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bats skills/autospec-shared/tests/unit/security-remediation-loop.bats`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-shared/scripts/security-remediation-loop.sh skills/autospec-shared/tests/unit/security-remediation-loop.bats
git commit -m "feat(secaudit): remediation-loop block/pass decision + secret rotation flag"
```

---

## Task 7: Create the autospec-secaudit skill trio (SKILL.md canonical body)

**Files:**
- Create: `skills/autospec-secaudit/SKILL.md`

> Write the canonical body here. Tasks 8–9 derive `codex/prompt.md` (byte-identical body, leading blank line) and `opencode/agent.md` (own frontmatter + identical body). Mirror `autospec-review/SKILL.md` section ordering. Every trio file MUST contain a `## Stop mode` heading (validate.sh enforces).

- [ ] **Step 1: Write SKILL.md**

```markdown
---
name: autospec-secaudit
description: Use when the user wants to scan generated code for security vulnerabilities, secret/credential leaks, IP/copyright/license violations, PII/data leaks, SQL injection, prompt injection, and backdoors. Runs as `/autospec-secaudit` (manual repo-wide sweep) or auto-fires after each autospec-run batch unless `~/.autospec/no-secaudit.flag` exists. Also powers the blocking Phase 4 per-PR gate.
---

<!-- BODY START -->
## Self-update mode

Decide this purely from the request text the harness handed you. Do NOT
shell out to test the user's free-form request. Read the request, normalize
it in your reasoning (collapse whitespace, trim, lowercase), and if the result is
exactly `update`, this skill enters self-update mode and does NOT run the
normal pipeline.

1. **Detect harness** by checking which install path exists for this skill:
   - Claude Code: `~/.claude/skills/autospec-secaudit/SKILL.md`
   - OpenCode:    `~/.config/opencode/agent/autospec-secaudit.md`
   - Codex CLI:   `~/.codex/prompts/autospec-secaudit.md`
2. **Re-install from `main`** by piping the canonical installer:
   ```bash
   curl -fsSL https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.sh | bash -s -- --skill all --harness all --update
   ```
   Run this one-liner once; it refreshes all autospec skills across all harnesses.
3. **Show the diff** between the prior installed file(s) and the freshly fetched copy.

## Stop mode

If the normalized request is exactly `stop` (or matches `^\s*stop(\s+--\w+)*\s*$`),
do NOT run the pipeline. Write `~/.autospec/stop.flag` so any running autospec
monitor halts after the current issue, then report that the stop sentinel was
written and exit.

## Required capabilities & harness adapter

| Capability                  | Claude Code                          | OpenCode                                 | Codex CLI                                | Fallback if missing                                |
|-----------------------------|--------------------------------------|------------------------------------------|------------------------------------------|----------------------------------------------------|
| Triage model tier            | Tier A: `opus` + ultrathink          | Tier A: top-tier `task` + max reasoning  | Tier A: `gpt-5.1-codex` + `reasoning_effort=high` | Fall back UP on unavailability |
| Deterministic scanners       | gitleaks, semgrep, trivy, license-checker via `ensure-tool.sh` | same | same | LLM-only fallback per missing scanner (loud WARN) |

## When to invoke

- Manually: `/autospec-secaudit` for a repo-wide sweep of the working tree.
- Automatically: auto-fires after each `/autospec-run` batch unless
  `~/.autospec/no-secaudit.flag` exists.
- As the Phase 4 gate engine (invoked by the implementer, not by a skill call).

## Pipeline

1. **Ensure scanners** — `ensure-tool.sh gitleaks semgrep trivy license-checker`
   (best-effort; a failed install degrades that concern to LLM-only with a loud
   WARN — never silently skipped).
2. **Scan** — run `security-scan.sh --tree` (sweep) or `--diff <base>` (branch).
   Findings are emitted as gap JSON objects:
   `{gap_id,dimension,severity,file,line,title,body,dedupe_key}`.
3. **LLM triage (Tier A)** — for each finding: confirm real vs false-positive;
   ADD findings scanners miss (PII in logs, prompt-injection sinks where
   untrusted input reaches an LLM call, contextual backdoors). Assign final
   `severity` (`must-fix` / `nice-to-have`).
4. **Report** — write `.autospec/secaudit.md` summarizing findings by dimension
   with file:line and remediation. Loudly list any scanner that fell back to
   LLM-only.
5. **File survivors** — must-fix survivors above threshold are filed as
   `auto-implement,security,priority:high` issues via the gap-filing path so
   they re-enter autospec. `dedupe_key` prevents re-filing across batches.

## Enforcement defaults

| Concern | dimension | Phase 4 gate | Auto-fix |
|---|---|---|---|
| Secrets / credentials | secrets | block | redact + flag rotation |
| Vulns / SQLi / cmd-injection / backdoors | vuln | block | yes, re-scan |
| Prompt injection | vuln | block | yes, re-scan |
| PII / data leaks | pii | block | yes, re-scan |
| IP / copyright / license | license | block | advisory fix (human confirms) |
| Dependency CVEs | cve | advisory (block only critical+fixable) | bump if trivial |

## Finalization

Print the report path and a one-line summary:
`secaudit: must-fix=<N> advisory=<N> scanners-degraded=<list>`.
```

- [ ] **Step 2: Verify frontmatter parses + Stop mode present**

Run: `grep -q '^## Stop mode' skills/autospec-secaudit/SKILL.md && head -3 skills/autospec-secaudit/SKILL.md`
Expected: prints the frontmatter opening; exit 0.

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-secaudit/SKILL.md
git commit -m "feat(secaudit): add autospec-secaudit SKILL.md (canonical body)"
```

---

## Task 8: Derive codex/prompt.md (lock-step byte-identical body)

**Files:**
- Create: `skills/autospec-secaudit/codex/prompt.md`

> `validate.sh check_lockstep()` byte-diffs `strip_body(SKILL.md)` against the raw `codex/prompt.md`. `strip_body` removes the YAML frontmatter and the `<!-- BODY START -->` line, leaving a **leading blank line** — so `codex/prompt.md` must start with one blank line then the `## Self-update mode` heading.

- [ ] **Step 1: Generate codex/prompt.md from SKILL.md**

Run (reproduces exactly what validate.sh compares against):

```bash
mkdir -p skills/autospec-secaudit/codex
awk 'f{print} /<!-- BODY START -->/{f=1}' skills/autospec-secaudit/SKILL.md > skills/autospec-secaudit/codex/prompt.md
```

- [ ] **Step 2: Confirm a leading blank line + first heading**

Run: `sed -n '1,3p' skills/autospec-secaudit/codex/prompt.md`
Expected: line 1 blank, line 2 `## Self-update mode`.

> If `strip_body` in `autospec validate` differs (e.g. it also strips the `<!-- BODY START -->` differently), match its exact transform. Inspect with: `grep -n 'strip_body' autospec validate` and read the function.

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-secaudit/codex/prompt.md
git commit -m "feat(secaudit): add codex/prompt.md (lock-step with SKILL.md)"
```

---

## Task 9: Derive opencode/agent.md + scaffolding (install/uninstall/README)

**Files:**
- Create: `skills/autospec-secaudit/opencode/agent.md`
- Create: `skills/autospec-secaudit/install.sh`
- Create: `skills/autospec-secaudit/uninstall.sh`
- Create: `skills/autospec-secaudit/README.md`

- [ ] **Step 1: Build opencode/agent.md (own frontmatter + identical body)**

OpenCode uses its own frontmatter; the body must match SKILL.md's body (validate.sh strips both bodies and diffs). Copy a sibling's frontmatter shape, then append the same body:

```bash
# Inspect the sibling's frontmatter shape first:
sed -n '1,12p' skills/autospec-review/opencode/agent.md
```

Then create `skills/autospec-secaudit/opencode/agent.md` with the OpenCode frontmatter (name/description/mode/model fields per the sibling), followed by the SKILL.md body (everything from `## Self-update mode` onward — same content as codex/prompt.md, preceded by the matching `<!-- BODY START -->` marker if the sibling includes it; match `strip_body` expectations exactly).

- [ ] **Step 2: Create install.sh / uninstall.sh by adapting the sibling**

```bash
sed 's/autospec-review/autospec-secaudit/g; s/AUTOSPEC_REVIEW/AUTOSPEC_SECAUDIT/g' \
  skills/autospec-review/install.sh > skills/autospec-secaudit/install.sh
sed 's/autospec-review/autospec-secaudit/g; s/AUTOSPEC_REVIEW/AUTOSPEC_SECAUDIT/g' \
  skills/autospec-review/uninstall.sh > skills/autospec-secaudit/uninstall.sh
chmod +x skills/autospec-secaudit/install.sh skills/autospec-secaudit/uninstall.sh
```

- [ ] **Step 3: Write README.md** — short: what the skill does, the six dimensions, the two entry points (manual sweep + Phase 4 gate), the env vars (`AUTOSPEC_SEC_MAX_ROUNDS`, `~/.autospec/no-secaudit.flag`, `AUTOSPEC_SKIP_ENSURE_TOOL_*`), and the report path `.autospec/secaudit.md`.

- [ ] **Step 4: Run lock-step validation**

Run: `autospec validate 2>&1 | grep -iE 'secaudit|lockstep|FAIL|PASS' | head`
Expected: no `secaudit ... diverges` failures. Fix body/marker mismatches until the trio passes lock-step.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-secaudit/opencode/agent.md skills/autospec-secaudit/install.sh skills/autospec-secaudit/uninstall.sh skills/autospec-secaudit/README.md
git commit -m "feat(secaudit): add opencode agent + install/uninstall/README scaffolding"
```

---

## Task 10: validate.sh named-content checks for the new trio

**Files:**
- Modify: `autospec validate`

- [ ] **Step 1: Read the existing named-content check pattern**

Run: `sed -n '118,160p' autospec validate`
Expected: shows the `## Stop mode` / `## Keyword auto-routing` invariant loops (the pattern to copy).

- [ ] **Step 2: Add a secaudit invariant block**

After the existing invariant loops, add a check that the secaudit trio carries the `## Enforcement defaults` heading in all three files (its skill-specific structural section), mirroring the Stop-mode loop:

```bash
# autospec-secaudit must carry an '## Enforcement defaults' heading in all
# three trio files so the concern→gate mapping stays in lock-step.
if [ -d "$SKILLS_DIR/autospec-secaudit" ]; then
    for trio in SKILL.md opencode/agent.md codex/prompt.md; do
        grep -q '^## Enforcement defaults' "$SKILLS_DIR/autospec-secaudit/$trio" \
            || fail "autospec-secaudit: $trio missing '## Enforcement defaults' section"
    done
fi
```

(Use the same `$SKILLS_DIR` / `fail` identifiers the surrounding code uses — confirm their exact names in context.)

- [ ] **Step 3: Run validate.sh**

Run: `autospec validate`
Expected: PASS (exit 0); the new trio passes lock-step + the new named check.

- [ ] **Step 4: Commit**

```bash
git add autospec validate
git commit -m "test(secaudit): validate.sh named-content check for enforcement-defaults section"
```

---

## Task 11: Wire the gate into Phase 4 (implementer prompt + contract)

**Files:**
- Modify: `skills/autospec-run/prompts/phase4-implementer.md` (around the pre-merge step near line 169–181)
- Modify: `skills/autospec-run/prompts/implementer-contract.md:66` (the `SECURITY` row)

> These are prompt files (instructions to the implementer subagent), not executable code — so the "test" is a grep assertion that the wiring text exists, plus a manual read-through. There is no bats harness for prompt prose; this matches how the rest of the run prompts are maintained.

- [ ] **Step 1: Add the gate step to phase4-implementer.md**

Immediately before the existing pre-merge peer-review/merge step, insert:

```markdown
### Security gate (blocking — before merge)

Before opening/merging the PR, run the security gate on the diff:

\```bash
bash "$AUTOSPEC_SHARED_SCRIPTS/security-remediation-loop.sh" --decide --diff main...HEAD --root .
\```

- If it prints `decision=pass` (exit 0) → continue to merge.
- If it prints `decision=block` (exit 1):
  - Treat each `must-fix` finding as a `SECURITY` directive. Fix the code
    (remove the vulnerable/leaking pattern; for a leaked secret, remove it AND
    note rotation in the PR body using any `ROTATE:` lines printed).
  - Re-run the gate. Repeat up to `AUTOSPEC_SEC_MAX_ROUNDS` (default 3) rounds.
  - If `must-fix` still survives after the cap → DO NOT merge. Post the findings
    to the PR and stop; surface to the operator.
- `nice-to-have` (advisory CVEs etc.) never blocks merge.
- If it prints `decision=block reason=engine-failed-closed` (exit 2) → DO NOT
  merge (fail-closed); report that the security engine could not run.
```

- [ ] **Step 2: Strengthen the SECURITY contract row**

Replace the `SECURITY` row in `implementer-contract.md:66` with:

```
| `SECURITY` | "Remove the flagged pattern. NEVER hardcode secrets (remove AND rotate), NEVER use --no-verify or git reset --hard, validate input at boundaries, parameterize SQL, never eval/exec untrusted input, never let untrusted input reach an LLM/prompt sink. The Phase 4 security gate (security-remediation-loop.sh) must report decision=pass before merge." |
```

- [ ] **Step 3: Verify the wiring text exists**

Run:
```bash
grep -q 'security-remediation-loop.sh' skills/autospec-run/prompts/phase4-implementer.md \
  && grep -q 'decision=pass before merge' skills/autospec-run/prompts/implementer-contract.md \
  && echo OK
```
Expected: `OK`.

- [ ] **Step 4: Confirm `$AUTOSPEC_SHARED_SCRIPTS` is defined for Phase 4**

Run: `grep -rn 'AUTOSPEC_SHARED_SCRIPTS' skills/autospec-run/`
Expected: at least one definition. If absent, add a resolution line near the top of `phase4-implementer.md` mirroring how other shared scripts are located (e.g. `AUTOSPEC_SHARED_SCRIPTS="${AUTOSPEC_SHARED_SCRIPTS:-$HOME/.claude/skills/autospec-shared/scripts}"`), matching the convention already used elsewhere in the run skill.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/prompts/phase4-implementer.md skills/autospec-run/prompts/implementer-contract.md
git commit -m "feat(secaudit): wire blocking security gate into Phase 4 before merge"
```

---

## Task 12: Auto-fire post-batch + full suite green

**Files:**
- Modify: `skills/autospec-run/scripts/run-batch-start.sh` OR the post-batch hook that triggers autospec-review (locate the autospec-review auto-fire and add secaudit beside it)

- [ ] **Step 1: Locate the existing post-batch auto-fire**

Run: `grep -rn 'no-review.flag\|autospec-review' skills/autospec-run/ skills/autospec-shared/scripts/run-batch-start.sh`
Expected: shows where autospec-review auto-fires after a batch (gated by `~/.autospec/no-review.flag`).

- [ ] **Step 2: Add the secaudit auto-fire beside it**

Mirror the autospec-review trigger, gated by its own flag:

```bash
# Auto-fire repo-wide security sweep after the batch unless opted out.
if [ ! -f "$HOME/.autospec/no-secaudit.flag" ]; then
  # invoke /autospec-secaudit (same mechanism the review auto-fire uses)
  ...
fi
```

Match the exact invocation mechanism the review auto-fire uses (skill dispatch vs script call) — copy its shape, swap the flag and skill name.

- [ ] **Step 3: Run the full security test suite**

Run:
```bash
bats skills/autospec-shared/tests/unit/ensure-tool-scanners.bats \
     skills/autospec-shared/tests/unit/security-scan.bats \
     skills/autospec-shared/tests/unit/security-remediation-loop.bats
```
Expected: all PASS.

- [ ] **Step 4: Run validate.sh end-to-end**

Run: `autospec validate`
Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/ skills/autospec-shared/
git commit -m "feat(secaudit): auto-fire repo-wide security sweep after each batch"
```

---

## Self-Review

**Spec coverage:**
- Two entry points sharing one engine → Tasks 3–6 (engine), 7–9 (skill), 11 (Phase 4 gate). ✓
- Six concern classes (secrets, vuln/SQLi/injection/backdoor, prompt-injection, PII, license/IP, CVE) → scanner paths Tasks 3–5 + LLM triage in SKILL.md pipeline; PII + prompt-injection are LLM-added per the pipeline (Task 7). ✓
- Deterministic-first + auto-install → Task 1 (ensure-tool) + scanner calls. ✓
- Auto-fix + re-scan loop, cap 3 → Task 6 (decision) + Task 11 (caller loop). ✓
- Secret rotation flag → Task 6 (`ROTATE:` lines) + Task 11 wiring. ✓
- CVE advisory → Task 5 (`nice-to-have` unless critical+fixable). ✓
- Auto-fire post-batch → Task 12. ✓
- Fail-closed engine / loud WARN on missing scanner → Tasks 3–4 tests. ✓
- gap JSON reuse + dedupe → Tasks 3/6 (gap-json-lib, dedupe_key). ✓
- Lock-step + validate.sh checks → Tasks 8–10. ✓
- TDD fixtures not self-derived → Task 2. ✓

**Placeholder scan:** scanner JSON shapes, gap object keys, exit codes, and flags are concrete. The two genuinely environment-dependent spots — `strip_body`'s exact transform (Task 8) and the post-batch auto-fire mechanism (Task 12) — are flagged as "inspect the sibling and match exactly" rather than guessed, because they must mirror existing repo internals the implementer will read in context.

**Type consistency:** dimension vocabulary (`secrets|vuln|injection|license|pii|cve`) and severity (`must-fix|nice-to-have`) are used identically across `security-scan.sh`, the loop, the SKILL.md table, and tests. Note: semgrep findings use dimension `vuln` (the `--only` filter accepts both `vuln` and `injection`; emitted objects standardize on `vuln`) — consistent across Tasks 4, 6, 7.

---

**Plan complete and saved to `docs/superpowers/plans/2026-06-14-autospec-secaudit.md`.**
