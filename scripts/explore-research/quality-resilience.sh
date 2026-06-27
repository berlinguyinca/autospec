#!/usr/bin/env bash
# scripts/explore-research/quality-resilience.sh — discovery researcher (quality-resilience).
#
# Four QA lenses:
#   (a) test files vs their SUT — flags self-consistent fixtures built with the
#       SUT's own derivation expression and assertion-free tests;
#   (b) each claimed invariant in validate.sh/SKILL prose vs whether a test AND
#       a guard exist;
#   (c) kill-mid-run / non-idempotent / shared-lock / partial-state hazards;
#   (d) LLM steps that should be deterministic + disproportionate-token phases.
#
# Cap: 100 candidates per round.
# Default weight: 0.95.
#
# Output: JSON to stdout matching schemas/autospec-explore-proposal.schema.json
# (extended contract with severity + named_consumer).

set -u

REPO_ROOT="${AUTOSPEC_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null || pwd)}"
# Cap lowered 100 -> 25: at 100 this researcher saturated its own cap every round
# (one self-declared silent-wrong flood swamping the severity-first rank). The
# assertion-density lens now also COLLAPSES en masse (see lens (a)).
MAX_PROPOSALS=25

cd "$REPO_ROOT" || { echo '{"source":"quality-resilience","proposals":[]}'; exit 0; }

if ! command -v python3 >/dev/null 2>&1; then
    echo '{"source":"quality-resilience","proposals":[]}'
    exit 0
fi

# Collect test files (bats + shell test helpers).
test_tmp="$(mktemp -t qr-tests.XXXXXX)"
trap 'rm -f "$test_tmp"' EXIT

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git ls-files -- '*.bats' 'tests/*.sh' 'test/*.sh' 'spec/*.sh' 2>/dev/null \
        | head -n 200 > "$test_tmp" || true
fi

# Collect validate.sh claim lines.
validate_tmp="$(mktemp -t qr-validate.XXXXXX)"
trap 'rm -f "$test_tmp" "$validate_tmp"' EXIT

if [ -f "scripts/validate.sh" ]; then
    grep -n 'check_\|assert\|must\|require\|ensure' scripts/validate.sh 2>/dev/null \
        | head -n 100 > "$validate_tmp" || true
fi

# Collect lock / partial-state hazard signals.
hazard_tmp="$(mktemp -t qr-hazard.XXXXXX)"
trap 'rm -f "$test_tmp" "$validate_tmp" "$hazard_tmp"' EXIT

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git grep -n -I -E '(lock|\.lock|lockfile|partial|PARTIAL|pid|PID|trap.*EXIT|rm.*tmp)' \
        -- 'scripts/*.sh' '*.sh' 2>/dev/null | head -n 200 > "$hazard_tmp" || true
fi

# Collect LLM-dispatch sites.
llm_tmp="$(mktemp -t qr-llm.XXXXXX)"
trap 'rm -f "$test_tmp" "$validate_tmp" "$hazard_tmp" "$llm_tmp"' EXIT

if command -v git >/dev/null 2>&1 && git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git grep -n -I -E '(claude|llm_dispatch|AskUserQuestion|invoke.*model|--model)' \
        -- 'scripts/*.sh' 'skills/**/*.md' 'skills/*.md' 2>/dev/null \
        | head -n 100 > "$llm_tmp" || true
fi

python3 - "$test_tmp" "$validate_tmp" "$hazard_tmp" "$llm_tmp" "$MAX_PROPOSALS" <<'PY'
import json, os, re, sys

test_path     = sys.argv[1]
validate_path = sys.argv[2]
hazard_path   = sys.argv[3]
llm_path      = sys.argv[4]
cap           = int(sys.argv[5])

proposals = []

def add(title, evidence, complexity="small", confidence=0.75,
        severity="correctness", named_consumer="", gap_check=None):
    if len(proposals) >= cap:
        return
    prop = {
        "title": title,
        "evidence": evidence,
        "estimated_complexity": complexity,
        "confidence": confidence,
        "severity": severity,
        "named_consumer": named_consumer,
    }
    if isinstance(gap_check, dict):
        prop["gap_check"] = gap_check
    proposals.append(prop)

# When more than this many test files trip the same lens, collapse to ONE
# structural proposal instead of one issue per file (anti-flood).
COLLAPSE_THRESHOLD = 8

def _bats_has_assertion(content):
    """True if a bats file contains ANY recognizable assertion — including
    NATIVE bats forms, not just the bats-assert `assert_*` helpers. The old
    `\\bassert\\b`-only check false-flagged every file that asserts with
    `[ ... ]` / `[[ ... ]]` / run-result vars, which is most of them."""
    return bool(re.search(
        r'\bassert\w*\b'                       # assert / assert_output / assert_success
        r'|\[\[?\s'                            # [ … ]  or  [[ … ]] test commands
        r'|\(\('                               # (( … )) arithmetic test
        r'|\$status\b|\$output\b|\$\{?lines\b'  # bats `run` result vars
        r'|\brefute\w*\b',                     # bats-assert refute_*
        content))

# ── Lens (a): assertion-free test files ────────────────────────────────────
try:
    with open(test_path, "r", encoding="utf-8", errors="ignore") as fh:
        test_files = [l.strip() for l in fh if l.strip()]
except FileNotFoundError:
    test_files = []

_assertion_free = []   # collect first, then collapse-or-emit below
for tf in test_files:
    if not os.path.isfile(tf):
        continue
    try:
        content = open(tf, "r", encoding="utf-8", errors="ignore").read()
    except Exception:
        continue

    # Flag bats files that have test blocks but NO recognizable assertion of
    # any kind (native or bats-assert). Files asserting via `[ … ]`/`$status`
    # are no longer false-flagged. The test-block marker is `@test` in source
    # files, or `bats_test_function` after bats' own preprocessing (the form a
    # fixture takes when created inside a bats heredoc).
    if tf.endswith(".bats"):
        has_test_block = ("@test" in content) or ("bats_test_function" in content)
        if has_test_block and not _bats_has_assertion(content):
            _assertion_free.append(tf)
            continue

    # Check for self-consistent fixture pattern: test imports / sources the SUT
    # and builds expected output via the same function call being tested.
    if re.search(r'source\s+\S+\s*\n.*expected.*=.*\$\(.*\)', content, re.DOTALL):
        add(
            f"test: replace self-consistent fixture in {tf} with pinned expected values",
            f"{tf} appears to derive expected output by calling the SUT — bugs in the SUT make fixtures self-validating (cf. lstrip transcript-slug bug).",
            complexity="small",
            confidence=0.7,
            severity="silent-wrong",
            named_consumer="/autospec-run mutation gate",
        )

# Collapse-or-emit the assertion-free bats files. Above COLLAPSE_THRESHOLD this
# is a systemic gap (one assertion-density-floor lint), not N separate issues —
# the prior per-file flood is exactly what saturated the cap. Each emitted
# proposal carries a gap_check confirming the file still has @test blocks.
if len(_assertion_free) > COLLAPSE_THRESHOLD:
    sample = ", ".join(_assertion_free[:5])
    add(
        f"test(structural): add an assertion-density floor lint ({len(_assertion_free)} assertion-free bats files)",
        f"{len(_assertion_free)} bats files have @test blocks but no recognizable "
        f"assertion (native `[ … ]`/`$status` or bats-assert). One lint that fails "
        f"CI on an assertion-free @test catches them all. Examples: {sample}.",
        complexity="medium",
        confidence=0.8,
        severity="silent-wrong",
        named_consumer="/autospec-run mutation gate; assertion-density floor",
        gap_check={"kind": "present", "needle": "@test", "haystack": _assertion_free[0]},
    )
else:
    for tf in _assertion_free:
        add(
            f"test: add assertions to {tf} (currently assertion-free)",
            f"{tf} contains @test blocks but no recognizable assertion (native "
            f"`[ … ]`/`$status` or bats-assert) — the test cannot falsify behaviour.",
            complexity="small",
            confidence=0.85,
            severity="silent-wrong",
            named_consumer="/autospec-run mutation gate; assertion-density floor",
            gap_check={"kind": "present", "needle": "@test", "haystack": tf},
        )

# ── Lens (b): validate.sh invariants without matching test ─────────────────
try:
    with open(validate_path, "r", encoding="utf-8", errors="ignore") as fh:
        validate_lines = fh.readlines()
except FileNotFoundError:
    validate_lines = []

check_fn_pat = re.compile(r'^(?:\d+:)?\s*(check_\w+)\s*\(\s*\)')
for i, line in enumerate(validate_lines, start=1):
    m = check_fn_pat.match(line)
    if not m:
        continue
    fn_name = m.group(1)
    # Heuristic: look for a bats @test that references this function name.
    found_test = False
    for tf in test_files:
        if not os.path.isfile(tf):
            continue
        try:
            tcontent = open(tf, "r", encoding="utf-8", errors="ignore").read()
            if fn_name in tcontent:
                found_test = True
                break
        except Exception:
            continue
    if not found_test:
        add(
            f"test: add bats coverage for validate.sh:{fn_name}",
            f"validate.sh defines {fn_name} (line {i}) but no bats test references it — the invariant is untested.",
            complexity="small",
            confidence=0.7,
            severity="correctness",
            named_consumer="/autospec-run validate gate; /autospec-sweep",
        )

# ── Lens (c): kill-mid-run / partial-state / shared-lock hazards ───────────
try:
    with open(hazard_path, "r", encoding="utf-8", errors="ignore") as fh:
        hazard_lines = fh.readlines()
except FileNotFoundError:
    hazard_lines = []

lock_files_seen = set()
for line in hazard_lines:
    line = line.strip()
    if not line:
        continue
    m = re.match(r'^(.+?):(\d+):(.+)$', line)
    if not m:
        continue
    fpath, lineno, content = m.group(1), m.group(2), m.group(3).strip()

    # Flag lockfile writes without a matching trap/cleanup.
    if re.search(r'\.lock\b', content) and fpath not in lock_files_seen:
        lock_files_seen.add(fpath)
        # Check if same file has a trap EXIT that removes the lock.
        try:
            full = open(fpath, "r", encoding="utf-8", errors="ignore").read()
            has_trap = bool(re.search(r'trap\s+.*rm.*\.lock|trap\s+.*cleanup', full))
        except Exception:
            has_trap = False
        if not has_trap:
            add(
                f"fix(resilience): ensure lockfile cleanup on exit in {fpath}",
                f"{fpath}:{lineno} uses a lockfile but no trap-on-EXIT cleanup found — kill-mid-run leaves stale locks.",
                complexity="small",
                confidence=0.72,
                severity="stability",
                named_consumer="/autospec-run concurrent-round guard",
            )

# ── Lens (d): LLM steps that could be deterministic ───────────────────────
try:
    with open(llm_path, "r", encoding="utf-8", errors="ignore") as fh:
        llm_lines = fh.readlines()
except FileNotFoundError:
    llm_lines = []

deterministic_candidates = set()
for line in llm_lines:
    line = line.strip()
    if not line:
        continue
    m = re.match(r'^(.+?):(\d+):(.+)$', line)
    if not m:
        continue
    fpath, lineno, content = m.group(1), m.group(2), m.group(3).strip()
    # If a script invokes claude but the surrounding content looks like a pure
    # grep/count operation, flag it.
    if re.search(r'(count|grep|find|ls|wc)\b', content) and re.search(r'claude|llm_dispatch', content):
        key = fpath
        if key not in deterministic_candidates:
            deterministic_candidates.add(key)
            add(
                f"refactor: replace LLM call with deterministic tool in {fpath}:{lineno}",
                f"{fpath}:{lineno} invokes an LLM for what appears to be a grep/count operation — convert to bash (cf. tooling-optimization tracker).",
                complexity="medium",
                confidence=0.6,
                severity="operability",
                named_consumer="autospec tooling-optimization tracker #421",
            )

print(json.dumps({"source": "quality-resilience", "proposals": proposals}))
PY
