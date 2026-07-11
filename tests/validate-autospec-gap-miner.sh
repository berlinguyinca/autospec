#!/usr/bin/env bash
# tests/validate-autospec-gap-miner.sh — deterministic coverage for autospec-gap-miner.sh.
set -eu

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

INPUT="$TMPDIR/gaps.txt"
LEDGER="$TMPDIR/autospec-gap-ledger.md"

cat > "$INPUT" <<'IN'
REQUEST_CHANGES: reviewer missed missing no-mock smoke coverage in skills/autospec-run/SKILL.md
FIX_COMMIT: follow-up commit fixed scope creep in scripts/validate.sh after review
CI_BLOCKER: pytest failed on tests/unit/example.bats and required a fix commit
REQUEST_CHANGES: reviewer missed missing no-mock smoke coverage in skills/autospec-run/SKILL.md
IN

out1="$TMPDIR/out1.json"
bash "$ROOT/scripts/autospec-gap-miner.sh" --input "$INPUT" --ledger "$LEDGER" --dry-run > "$out1"

jq -e 'length == 3' "$out1" >/dev/null
jq -e 'map(.kind) | sort == ["ci_blocker","fix_commit","request_changes"]' "$out1" >/dev/null
jq -e 'all(.[]; (.labels | index("gap-remediation")) and ((.labels | map(startswith("area:")) | map(select(.)) | length) == 1))' "$out1" >/dev/null
jq -e 'any(.[]; .source_type == "REQUEST_CHANGES")' "$out1" >/dev/null
jq -e 'any(.[]; .source_type == "FIX_COMMIT")' "$out1" >/dev/null
jq -e 'any(.[]; .source_type == "CI_BLOCKER")' "$out1" >/dev/null

grep -q '^| dedupe_key | kind | area | repeat_count | priority | last_seen |$' "$LEDGER"
grep -q '| request-changes-reviewer-missed-missing-no-mock-smoke-coverage-in-skills-autospec-run-skill-md | request_changes | area:review | 1 | priority:medium |' "$LEDGER"

# A repeat miss increments the stable ledger key and raises priority on the next draft.
out2="$TMPDIR/out2.json"
printf '%s\n' 'REQUEST_CHANGES: reviewer missed missing no-mock smoke coverage in skills/autospec-run/SKILL.md' \
  | bash "$ROOT/scripts/autospec-gap-miner.sh" --input - --ledger "$LEDGER" --dry-run > "$out2"
jq -e '.[0].priority == "priority:high"' "$out2" >/dev/null
grep -q '| request-changes-reviewer-missed-missing-no-mock-smoke-coverage-in-skills-autospec-run-skill-md | request_changes | area:review | 2 | priority:high |' "$LEDGER"

# Filing mode must dedupe with gh issue list --search before gh issue create.
GH_LOG="$TMPDIR/gh.log"
mkdir -p "$TMPDIR/bin"
cat > "$TMPDIR/bin/gh" <<'GH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
if [ "$1 $2" = "issue list" ]; then
  printf '[]\n'
  exit 0
fi
if [ "$1 $2" = "issue create" ]; then
  printf 'https://github.com/example/repo/issues/99\n'
  exit 0
fi
if [ "$1 $2" = "label create" ]; then
  exit 0
fi
exit 1
GH
chmod +x "$TMPDIR/bin/gh"
PATH="$TMPDIR/bin:$PATH" GH_LOG="$GH_LOG" bash "$ROOT/scripts/autospec-gap-miner.sh" --input "$INPUT" --ledger "$TMPDIR/file-ledger.md" --repo example/repo --file > "$TMPDIR/file.json"

grep -q -- 'issue list .*--search' "$GH_LOG"
grep -q -- 'issue create ' "$GH_LOG"
list_line="$(grep -n -- 'issue list .*--search' "$GH_LOG" | head -1 | cut -d: -f1)"
create_line="$(grep -n -- 'issue create ' "$GH_LOG" | head -1 | cut -d: -f1)"
[ "$list_line" -lt "$create_line" ]
