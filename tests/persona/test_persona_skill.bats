#!/usr/bin/env bats
# tests/persona/test_persona_skill.bats — contract gate for the
# /autospec-persona calibration-interview skill (issue #1418, feature F5).
#
# The skill itself is authored prose (a trio), but it commits to a deterministic
# data contract:
#   - the ~/.autospec/operator-persona.answers.json schema (version 1),
#   - a HARD cap of ≤50 questions,
#   - resume strictly at next_batch (a done batch is never re-asked),
#   - calibration-agreement % computed over MULTIPLE-CHOICE calibration questions
#     only (free-text excluded).
#
# These tests encode that contract as small jq reference computations and assert
# them against fixtures with KNOWN expected values. The fixtures are hand-written
# (NOT derived from the helper under test), so a wrong helper cannot make a wrong
# fixture pass — guarding against the self-consistent-fixture failure mode.
#
# Conventions (repo bash 3.2 + macOS gotchas):
#   - every fixture is written to a REAL temp file before any [ -f ] / jq read
#     (process-substitution `[ -f <(...) ]` is false on bash 3.2);
#   - no RETURN traps; if/then/fi for one-sided conditionals.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SKILL_DIR="$REPO_ROOT/skills/autospec-persona"

setup() {
    TMP="$(mktemp -d -t persona.XXXXXX)"
}

teardown() {
    if [ -n "${TMP:-}" ] && [ -d "$TMP" ]; then
        rm -rf "$TMP"
    fi
}

# ── reference contract helpers (the "SUT" for the data contract) ─────────────

# Total questions across every batch.
persona_total_questions() {  # persona_total_questions <answers-file>
    jq '[.batches[].questions[]] | length' "$1"
}

# next_batch per the contract: index of the first batch whose status != "done",
# or the batch count when every batch is done. Computed independently of the
# value stored in the file so a test can cross-check the stored value.
persona_compute_next_batch() {  # persona_compute_next_batch <answers-file>
    jq '
      (.batches | length) as $n
      | ([ .batches | to_entries[] | select(.value.status != "done") | .key ]) as $pending
      | (if ($pending | length) == 0 then $n else $pending[0] end)
    ' "$1"
}

# Whether every batch BEFORE next_batch is done (i.e. resume never re-asks a done
# batch and never skips a pending one). Echoes "ok" or "bad".
persona_resume_consistent() {  # persona_resume_consistent <answers-file>
    jq -r '
      .next_batch as $nb
      | [ .batches[0:$nb][]? | select(.status != "done") ] as $earlypending
      | if ($earlypending | length) == 0 then "ok" else "bad" end
    ' "$1"
}

# Calibration-agreement % over MULTIPLE-CHOICE calibration records only.
# Operates on an in-flight calibration record array: each entry carries
# {choice, free, is_calibration, prediction}. A multiple-choice record has a
# non-null choice; free-text calibration (choice == null) is excluded.
persona_agreement_pct() {  # persona_agreement_pct <calibration-file>
    jq '
      ([ .[] | select(.is_calibration == true and .choice != null) ]) as $mc
      | ($mc | length) as $tot
      | (if $tot == 0 then 0
         else (([ $mc[] | select(.choice == .prediction) ] | length) * 100 / $tot)
         end)
    ' "$1"
}

# Build an answers file with N single-question batches into <file>.
persona_make_answers_with_n() {  # persona_make_answers_with_n <n> <file>
    jq -n --argjson n "$1" '
      {
        version: 1,
        batches: [ range(0; $n) | { id: ("b\(.)"), status: "done",
                    questions: [ { q: "q\(.)", choice: "a", free: null } ] } ],
        next_batch: $n
      }
    ' > "$2"
}

# ── tests ────────────────────────────────────────────────────────────────

@test "trio + goldens are present for autospec-persona" {
    [ -f "$SKILL_DIR/SKILL.md" ]
    [ -f "$SKILL_DIR/codex/prompt.md" ]
    [ -f "$SKILL_DIR/opencode/agent.md" ]
    [ -f "$REPO_ROOT/tests/fixtures/skill-goldens/autospec-persona.SKILL.md.sha256" ]
}

@test "SKILL.md declares the ≤50 hard cap" {
    grep -q '50' "$SKILL_DIR/SKILL.md"
    grep -qi 'hard cap' "$SKILL_DIR/SKILL.md"
}

@test "answers-file shape matches the version-1 schema" {
    f="$TMP/answers.json"
    cat > "$f" <<'JSON'
{
  "version": 1,
  "batches": [
    { "id": "merge-autonomy", "status": "done",
      "questions": [ { "q": "Auto-merge clean PRs?", "choice": "auto-merge", "free": null } ] },
    { "id": "test-discipline", "status": "pending",
      "questions": [ { "q": "Flaky test policy?", "choice": null, "free": "quarantine then fix" } ] }
  ],
  "next_batch": 1
}
JSON
    [ -f "$f" ]
    run jq -e '
      .version == 1
      and (.batches | type == "array")
      and (.next_batch | type == "number")
      and ([ .batches[] | has("id") and has("status") and has("questions") ] | all)
      and ([ .batches[].questions[] | has("q") and has("choice") and has("free") ] | all)
      and ([ .batches[].status | . == "done" or . == "pending" ] | all)
    ' "$f"
    [ "$status" -eq 0 ]
}

@test "≤50-question cap: a 50-question file is within the cap" {
    f="$TMP/cap50.json"
    persona_make_answers_with_n 50 "$f"
    [ -f "$f" ]
    run persona_total_questions "$f"
    [ "$status" -eq 0 ]
    [ "$output" -eq 50 ]
    [ "$output" -le 50 ]
}

@test "≤50-question cap: a 51-question file violates the cap" {
    f="$TMP/cap51.json"
    persona_make_answers_with_n 51 "$f"
    [ -f "$f" ]
    run persona_total_questions "$f"
    [ "$status" -eq 0 ]
    [ "$output" -eq 51 ]
    [ "$output" -gt 50 ]
}

@test "resume: next_batch points at the first pending batch (done batches skipped)" {
    f="$TMP/resume.json"
    cat > "$f" <<'JSON'
{
  "version": 1,
  "batches": [
    { "id": "b0", "status": "done",    "questions": [ { "q": "q0", "choice": "a", "free": null } ] },
    { "id": "b1", "status": "done",    "questions": [ { "q": "q1", "choice": "b", "free": null } ] },
    { "id": "b2", "status": "pending", "questions": [ { "q": "q2", "choice": null, "free": null } ] },
    { "id": "b3", "status": "pending", "questions": [ { "q": "q3", "choice": null, "free": null } ] }
  ],
  "next_batch": 2
}
JSON
    [ -f "$f" ]
    # The computed next_batch matches the stored one.
    run persona_compute_next_batch "$f"
    [ "$status" -eq 0 ]
    [ "$output" -eq 2 ]
    # And resume is consistent: no pending batch sits before next_batch, so a
    # done batch is never re-asked.
    run persona_resume_consistent "$f"
    [ "$status" -eq 0 ]
    [ "$output" = "ok" ]
}

@test "resume: a fully-done interview reports the terminal next_batch" {
    f="$TMP/done.json"
    persona_make_answers_with_n 3 "$f"   # all done, next_batch = 3
    [ -f "$f" ]
    run persona_compute_next_batch "$f"
    [ "$status" -eq 0 ]
    [ "$output" -eq 3 ]
}

@test "calibration: agreement % is computed over multiple-choice only" {
    f="$TMP/calib.json"
    # 4 MC calibration records (3 match), 1 free-text calibration (excluded),
    # 1 non-calibration MC (excluded). Expected: 3/4 = 75. If the free-text or
    # the non-calibration record leaked into the denominator the result would
    # not be exactly 75.
    cat > "$f" <<'JSON'
[
  { "choice": "a", "free": null, "is_calibration": true,  "prediction": "a" },
  { "choice": "b", "free": null, "is_calibration": true,  "prediction": "b" },
  { "choice": "c", "free": null, "is_calibration": true,  "prediction": "c" },
  { "choice": "d", "free": null, "is_calibration": true,  "prediction": "x" },
  { "choice": null, "free": "open answer", "is_calibration": true, "prediction": "z" },
  { "choice": "a", "free": null, "is_calibration": false, "prediction": "a" }
]
JSON
    [ -f "$f" ]
    run persona_agreement_pct "$f"
    [ "$status" -eq 0 ]
    [ "$output" -eq 75 ]
}

@test "calibration: no multiple-choice calibration questions yields 0%" {
    f="$TMP/calib-empty.json"
    cat > "$f" <<'JSON'
[
  { "choice": null, "free": "open", "is_calibration": true, "prediction": "z" },
  { "choice": "a", "free": null, "is_calibration": false, "prediction": "a" }
]
JSON
    [ -f "$f" ]
    run persona_agreement_pct "$f"
    [ "$status" -eq 0 ]
    [ "$output" -eq 0 ]
}
