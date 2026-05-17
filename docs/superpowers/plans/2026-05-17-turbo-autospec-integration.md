# Turbo ↔ Autospec Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Absorb turbo's high-leverage practices (peer-review via Codex, expand-before-implement, finalize QA gate, spec self-review) into autospec without forking any turbo skill.

**Architecture:** Zero forked skills. Three phases, each independently shippable: (A) `install.sh` extended to bootstrap turbo + check Codex + merge CLAUDE.md, (B) `/autospec-define` adopts spec self-review pass + shell-structure decomposer questions + `autospec:v2-flow` label on filed issues, (C) new Phase 4 implementer prompt template at `skills/autospec-run/prompts/phase4-implementer.md` that embeds expand → implement → finalize → peer-review → evaluate-findings inline.

**Tech Stack:** bash 3.2+ (macOS-compatible), gh CLI, Codex CLI (optional, graceful skip if absent), markdown prompt files.

**Spec:** `docs/superpowers/specs/2026-05-17-turbo-autospec-integration-design.md`

---

## File Structure

**Phase A — install.sh extensions** (changes to existing files only):
- Modify: `install.sh` — add turbo bootstrap, Codex check, CLAUDE.md merge, `.autospec/` gitignore offer, `--update`, `--check`, `--uninstall` flags
- Modify: `README.md` — install/update section
- Create: `scripts/lib/install-helpers.sh` — reusable functions for the install steps (idempotent CLAUDE.md merge, symlink helpers)
- Create: `tests/install/test_install_dry_run.sh` — dry-run assertion tests

**Phase B — /autospec-define changes**:
- Modify: `skills/autospec-define/SKILL.md` — add spec self-review pass section, shell-structure decomposer questions, `autospec:v2-flow` label on issue filing

**Phase C — Phase 4 implementer prompt**:
- Create: `skills/autospec-run/prompts/phase4-implementer.md` — the absorbed-discipline prompt template
- Modify: `skills/autospec-run/SKILL.md` — route Phase 4 monitor subagent to the new prompt when issue carries `autospec:v2-flow` label
- Create: `tests/phase4/test_prompt_structure.sh` — verify prompt file has all required sections

---

## Phase A — install.sh extensions

Goal of phase: one command (`install.sh --update`) keeps both autospec and turbo current. Operators new to autospec get turbo installed automatically.

### Task A1: Create reusable install helpers library

**Files:**
- Create: `scripts/lib/install-helpers.sh`
- Test: `tests/install/test_helpers.sh`

- [ ] **Step 1: Write the failing test**

Create `tests/install/test_helpers.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$SCRIPT_DIR/scripts/lib/install-helpers.sh"

# Test: merge_marked_block writes a fenced block into a target file idempotently
tmpfile=$(mktemp)
echo "existing content" > "$tmpfile"

merge_marked_block "$tmpfile" "autospec-block" "test content"
grep -q "<!-- autospec-block -->" "$tmpfile" || { echo "FAIL: marker not added"; exit 1; }
grep -q "test content" "$tmpfile" || { echo "FAIL: content not added"; exit 1; }

# Re-running with same content must not duplicate
merge_marked_block "$tmpfile" "autospec-block" "test content"
count=$(grep -c "<!-- autospec-block -->" "$tmpfile")
[[ "$count" -eq 1 ]] || { echo "FAIL: marker duplicated (count=$count)"; exit 1; }

# Re-running with different content must replace in place
merge_marked_block "$tmpfile" "autospec-block" "updated content"
grep -q "updated content" "$tmpfile" || { echo "FAIL: content not updated"; exit 1; }
! grep -q "test content" "$tmpfile" || { echo "FAIL: old content not removed"; exit 1; }

rm "$tmpfile"
echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tests/install/test_helpers.sh`
Expected: FAIL with "install-helpers.sh: No such file or directory"

- [ ] **Step 3: Write the helpers library**

Create `scripts/lib/install-helpers.sh`:

```bash
#!/usr/bin/env bash
# Reusable install helpers. Source this file; do not execute.

# Idempotently merge a fenced block into a target file.
# Usage: merge_marked_block <file> <marker-name> <content>
# Writes:
#   <!-- marker-name -->
#   content
#   <!-- /marker-name -->
# If the block exists, it is replaced in place. If not, it is appended.
merge_marked_block() {
    local file="$1"
    local marker="$2"
    local content="$3"
    local start="<!-- ${marker} -->"
    local end="<!-- /${marker} -->"
    local tmp
    tmp=$(mktemp)

    [[ -f "$file" ]] || touch "$file"

    if grep -q "$start" "$file"; then
        awk -v s="$start" -v e="$end" -v c="$content" '
            $0 == s { print; print c; in_block=1; next }
            $0 == e { in_block=0; print; next }
            !in_block { print }
        ' "$file" > "$tmp"
    else
        cat "$file" > "$tmp"
        {
            [[ -s "$tmp" ]] && echo ""
            echo "$start"
            echo "$content"
            echo "$end"
        } >> "$tmp"
    fi

    mv "$tmp" "$file"
}

# Check if a command exists on PATH.
# Usage: command_present <name>
command_present() {
    command -v "$1" >/dev/null 2>&1
}

# Ensure a line is present in a file. Idempotent.
# Usage: ensure_line_in_file <file> <line>
ensure_line_in_file() {
    local file="$1"
    local line="$2"
    [[ -f "$file" ]] || touch "$file"
    grep -qxF "$line" "$file" || echo "$line" >> "$file"
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tests/install/test_helpers.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add scripts/lib/install-helpers.sh tests/install/test_helpers.sh
git commit -m "feat(install): add reusable install-helpers library"
```

### Task A2: Add turbo bootstrap to install.sh

**Files:**
- Modify: `install.sh` (add bootstrap_turbo function and call site)
- Test: `tests/install/test_install_dry_run.sh`

- [ ] **Step 1: Write the failing test**

Create `tests/install/test_install_dry_run.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Dry-run mode must report bootstrap_turbo step without executing it
output=$(AUTOSPEC_DRY_RUN=1 bash "$SCRIPT_DIR/install.sh" --check 2>&1 || true)
echo "$output" | grep -q "bootstrap_turbo" || { echo "FAIL: bootstrap step not reported"; echo "$output"; exit 1; }
echo "$output" | grep -q "tobihagemann/turbo" || { echo "FAIL: turbo repo not mentioned"; echo "$output"; exit 1; }

# Dry-run must not actually clone
[[ ! -d "$HOME/.turbo/repo-test-marker" ]] || { echo "FAIL: dry-run made changes"; exit 1; }

echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tests/install/test_install_dry_run.sh`
Expected: FAIL with "bootstrap step not reported" or similar

- [ ] **Step 3: Add bootstrap_turbo to install.sh**

Open `install.sh`. After existing argument parsing, source the helpers library:

```bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/install-helpers.sh
source "$SCRIPT_DIR/scripts/lib/install-helpers.sh"

AUTOSPEC_DRY_RUN="${AUTOSPEC_DRY_RUN:-0}"

run_or_report() {
    local description="$1"; shift
    if [[ "$AUTOSPEC_DRY_RUN" == "1" ]]; then
        echo "[dry-run] $description: $*"
    else
        echo "$description"
        "$@"
    fi
}

bootstrap_turbo() {
    local turbo_repo="$HOME/.turbo/repo"
    if [[ -d "$turbo_repo/.git" ]]; then
        run_or_report "bootstrap_turbo: pulling tobihagemann/turbo" \
            git -C "$turbo_repo" pull --ff-only
    else
        run_or_report "bootstrap_turbo: cloning tobihagemann/turbo to $turbo_repo" \
            git clone --depth 1 https://github.com/tobihagemann/turbo.git "$turbo_repo"
    fi

    if [[ "$AUTOSPEC_DRY_RUN" == "1" ]]; then
        echo "[dry-run] bootstrap_turbo: would symlink turbo skills into ~/.claude/skills/"
        return
    fi

    local turbo_skills="$turbo_repo/claude/skills"
    local target_skills="$HOME/.claude/skills"
    mkdir -p "$target_skills"
    [[ -d "$turbo_skills" ]] || { echo "WARN: $turbo_skills not found; turbo install layout changed?"; return; }
    local skill
    for skill in "$turbo_skills"/*; do
        [[ -d "$skill" ]] || continue
        local name
        name=$(basename "$skill")
        ln -sfn "$skill" "$target_skills/$name"
    done
}
```

Then add a call to `bootstrap_turbo` in the main install flow, before the existing autospec skill symlinking.

Add argument parsing for `--check`:

```bash
case "${1:-}" in
    --check) AUTOSPEC_DRY_RUN=1 ;;
    --update) ;;
    --uninstall) exec bash "$SCRIPT_DIR/uninstall.sh" ;;
    "") ;;
    *) echo "Unknown flag: $1"; exit 1 ;;
esac
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tests/install/test_install_dry_run.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add install.sh tests/install/test_install_dry_run.sh
git commit -m "feat(install): bootstrap tobihagemann/turbo on install"
```

### Task A3: Add Codex CLI check with graceful guidance

**Files:**
- Modify: `install.sh`
- Test: `tests/install/test_codex_check.sh`

- [ ] **Step 1: Write the failing test**

Create `tests/install/test_codex_check.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Force codex absence via PATH manipulation
output=$(PATH="/usr/bin:/bin" AUTOSPEC_DRY_RUN=1 bash "$SCRIPT_DIR/install.sh" --check 2>&1 || true)
echo "$output" | grep -qi "codex" || { echo "FAIL: codex absence not mentioned"; echo "$output"; exit 1; }
echo "$output" | grep -qi "peer-review will skip" || { echo "FAIL: graceful-skip note missing"; echo "$output"; exit 1; }

echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tests/install/test_codex_check.sh`
Expected: FAIL with "codex absence not mentioned"

- [ ] **Step 3: Add check_codex to install.sh**

Append to `install.sh`:

```bash
check_codex() {
    if command_present codex; then
        echo "check_codex: codex CLI present ($(command -v codex))"
        return 0
    fi
    cat <<'EOF'
check_codex: codex CLI NOT found on PATH.
  Phase 4 peer-review will skip gracefully until codex is installed.
  Install: see https://github.com/openai/codex (or your package manager).
EOF
    return 0
}
```

Call `check_codex` from the main install flow after `bootstrap_turbo`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tests/install/test_codex_check.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add install.sh tests/install/test_codex_check.sh
git commit -m "feat(install): check Codex CLI presence, document graceful skip"
```

### Task A4: Merge autospec block into ~/.claude/CLAUDE.md

**Files:**
- Modify: `install.sh`
- Create: `scripts/lib/claude-md-block.txt` — the canonical block content
- Test: `tests/install/test_claude_md_merge.sh`

- [ ] **Step 1: Write the failing test**

Create `tests/install/test_claude_md_merge.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$SCRIPT_DIR/scripts/lib/install-helpers.sh"

fake_home=$(mktemp -d)
mkdir -p "$fake_home/.claude"
echo "# Existing content" > "$fake_home/.claude/CLAUDE.md"

HOME="$fake_home" bash "$SCRIPT_DIR/install.sh" >/dev/null

grep -q "<!-- autospec-block -->" "$fake_home/.claude/CLAUDE.md" || { echo "FAIL: marker missing"; exit 1; }
grep -q "Existing content" "$fake_home/.claude/CLAUDE.md" || { echo "FAIL: existing content lost"; exit 1; }

# Re-run must be idempotent
HOME="$fake_home" bash "$SCRIPT_DIR/install.sh" >/dev/null
count=$(grep -c "<!-- autospec-block -->" "$fake_home/.claude/CLAUDE.md")
[[ "$count" -eq 1 ]] || { echo "FAIL: marker duplicated (count=$count)"; exit 1; }

rm -rf "$fake_home"
echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tests/install/test_claude_md_merge.sh`
Expected: FAIL with "marker missing"

- [ ] **Step 3: Create the canonical block content**

Create `scripts/lib/claude-md-block.txt`:

```
## autospec + turbo integration

This machine has both **autospec** and **turbo** skill families installed.

**autospec entrypoints (autonomous, multi-issue pipelines):**
- `/autospec-define` — plan a feature, decompose into GitHub issues
- `/autospec-run` — run the implementation loop with auto-merge
- `/autospec-review`, `/autospec-classify`, `/autospec-split`, `/autospec-story` — auxiliary skills

**turbo entrypoints (interactive, single-task discipline):**
- `/turboplan`, `/draft-spec`, `/finalize`, `/peer-review`, `/self-improve`

**Per-issue Phase 4 implementer** (inside `/autospec-run`) absorbs turbo's expand → implement → finalize → peer-review → evaluate discipline inline. No skill calls from within the Phase 4 subagent.

**Update both stacks**: `bash <autospec-repo>/install.sh --update`

**Codex CLI** powers `/peer-review`. If absent, Phase 4 peer-review skips gracefully.
```

- [ ] **Step 4: Add merge_claude_md to install.sh**

Append to `install.sh`:

```bash
merge_claude_md() {
    local claude_md="$HOME/.claude/CLAUDE.md"
    local block_file="$SCRIPT_DIR/scripts/lib/claude-md-block.txt"
    [[ -f "$block_file" ]] || { echo "WARN: $block_file missing; skipping CLAUDE.md merge"; return; }

    if [[ "$AUTOSPEC_DRY_RUN" == "1" ]]; then
        echo "[dry-run] merge_claude_md: would update <!-- autospec-block --> in $claude_md"
        return
    fi

    mkdir -p "$(dirname "$claude_md")"
    local content
    content=$(cat "$block_file")
    merge_marked_block "$claude_md" "autospec-block" "$content"
    echo "merge_claude_md: updated $claude_md"
}
```

Call `merge_claude_md` from the main install flow after `check_codex`.

- [ ] **Step 5: Run test to verify it passes**

Run: `bash tests/install/test_claude_md_merge.sh`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add install.sh scripts/lib/claude-md-block.txt tests/install/test_claude_md_merge.sh
git commit -m "feat(install): idempotent CLAUDE.md merge with autospec-block marker"
```

### Task A5: Offer .autospec/ gitignore in current repo

**Files:**
- Modify: `install.sh`
- Test: `tests/install/test_gitignore_offer.sh`

- [ ] **Step 1: Write the failing test**

Create `tests/install/test_gitignore_offer.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$SCRIPT_DIR/scripts/lib/install-helpers.sh"

# Create a throwaway git repo
tmprepo=$(mktemp -d)
git -C "$tmprepo" init -q
touch "$tmprepo/.gitignore"

(cd "$tmprepo" && AUTOSPEC_AUTO_YES=1 bash "$SCRIPT_DIR/install.sh" >/dev/null)
grep -qxF ".autospec/" "$tmprepo/.gitignore" || { echo "FAIL: .autospec/ not added"; exit 1; }

# Re-run must be idempotent
(cd "$tmprepo" && AUTOSPEC_AUTO_YES=1 bash "$SCRIPT_DIR/install.sh" >/dev/null)
count=$(grep -cxF ".autospec/" "$tmprepo/.gitignore")
[[ "$count" -eq 1 ]] || { echo "FAIL: .autospec/ duplicated"; exit 1; }

rm -rf "$tmprepo"
echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tests/install/test_gitignore_offer.sh`
Expected: FAIL

- [ ] **Step 3: Add offer_gitignore to install.sh**

Append to `install.sh`:

```bash
offer_gitignore() {
    git rev-parse --is-inside-work-tree >/dev/null 2>&1 || return 0
    local gitignore
    gitignore="$(git rev-parse --show-toplevel)/.gitignore"
    local entry=".autospec/"

    if [[ -f "$gitignore" ]] && grep -qxF "$entry" "$gitignore"; then
        return 0
    fi

    if [[ "${AUTOSPEC_AUTO_YES:-0}" == "1" ]]; then
        ensure_line_in_file "$gitignore" "$entry"
        echo "offer_gitignore: added $entry to $gitignore"
        return 0
    fi

    read -r -p "Add '.autospec/' to $gitignore? [y/N] " reply
    case "$reply" in
        [yY]*)
            ensure_line_in_file "$gitignore" "$entry"
            echo "offer_gitignore: added $entry to $gitignore"
            ;;
        *) echo "offer_gitignore: skipped" ;;
    esac
}
```

Call `offer_gitignore` from the main install flow last.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tests/install/test_gitignore_offer.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add install.sh tests/install/test_gitignore_offer.sh
git commit -m "feat(install): offer .autospec/ gitignore entry in current repo"
```

### Task A6: Wire --update flag

**Files:**
- Modify: `install.sh`
- Test: `tests/install/test_update_flag.sh`

- [ ] **Step 1: Write the failing test**

Create `tests/install/test_update_flag.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

output=$(AUTOSPEC_DRY_RUN=1 bash "$SCRIPT_DIR/install.sh" --update 2>&1 || true)
echo "$output" | grep -qi "git pull" || { echo "FAIL: --update did not invoke git pull"; echo "$output"; exit 1; }
echo "$output" | grep -qi "autospec" || { echo "FAIL: --update did not mention autospec"; exit 1; }
echo "$output" | grep -qi "turbo" || { echo "FAIL: --update did not mention turbo"; exit 1; }

echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tests/install/test_update_flag.sh`
Expected: FAIL

- [ ] **Step 3: Wire --update to pull both repos**

In `install.sh`, add an `update_autospec` function:

```bash
update_autospec() {
    run_or_report "update_autospec: pulling autospec" \
        git -C "$SCRIPT_DIR" pull --ff-only
    bootstrap_turbo   # already does pull-if-exists, clone-if-not
}
```

Update the case statement so `--update` calls `update_autospec` before the rest of the install flow:

```bash
case "${1:-}" in
    --check) AUTOSPEC_DRY_RUN=1 ;;
    --update) update_autospec ;;
    --uninstall) exec bash "$SCRIPT_DIR/uninstall.sh" ;;
    "") ;;
    *) echo "Unknown flag: $1"; exit 1 ;;
esac
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tests/install/test_update_flag.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add install.sh tests/install/test_update_flag.sh
git commit -m "feat(install): --update pulls both autospec and turbo"
```

### Task A7: Document install/update in README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add Install / Update section to README.md**

Find the existing install section in `README.md`. Replace or append:

```markdown
## Install / Update

Single-command install (also installs turbo as a peer skill family):

    bash install.sh

Keep both autospec and turbo current:

    bash install.sh --update

Dry-run report (no changes):

    bash install.sh --check

Uninstall:

    bash install.sh --uninstall

`install.sh` bootstraps `tobihagemann/turbo` into `~/.turbo/repo`, symlinks both skill families into `~/.claude/skills/`, checks for the Codex CLI (used by Phase 4 peer-review; graceful skip if absent), idempotently merges an `<!-- autospec-block -->` section into `~/.claude/CLAUDE.md`, and offers to add `.autospec/` to the current repo's `.gitignore`.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs(install): document --update / --check flags + turbo bootstrap"
```

**Phase A ship checkpoint**: at this point `install.sh` is feature-complete and tested. Open PR for Phase A before proceeding to Phase B.

---

## Phase B — /autospec-define adopts spec self-review + v2-flow label

Goal of phase: `/autospec-define` produces issues that opt into the new Phase 4 implementer (via the `autospec:v2-flow` label) and runs a spec self-review pass before filing. The decomposer asks itself shell-structure questions (Produces/Consumes/Covers) but does not emit YAML — those answers shape decomposition quality only.

### Task B1: Add spec self-review pass section to /autospec-define

**Files:**
- Modify: `skills/autospec-define/SKILL.md`

- [ ] **Step 1: Read existing /autospec-define structure**

Run: `head -200 skills/autospec-define/SKILL.md`
Identify the boundary between "spec drafted" and "issues filed" — this is where the self-review pass inserts.

- [ ] **Step 2: Insert the self-review pass section**

After the spec-writing phase, before the decomposition phase, add a `### Spec self-review pass (Phase 2.5)` section:

```markdown
### Phase 2.5 — Spec self-review pass

Before decomposing into issues, re-read the spec with fresh eyes and check:

1. **Placeholders**: Search for "TBD", "TODO", "later", "tbd", "fixme". Fill in or remove. If a placeholder is intentional (genuine open question for the operator), surface it via AskUserQuestion now — do not file an issue against an undecided spec.
2. **Internal consistency**: Do any two sections contradict each other? Does the architecture match the feature description? Fix inline.
3. **Ambiguity**: Could any requirement be interpreted two different ways? Pick one and make it explicit.
4. **Scope**: Is this focused enough for a single multi-issue pipeline, or does it need decomposition into sub-specs (each with its own /autospec-define run)? If decomposition is needed, stop and surface this to the operator.

This pass is intentionally lightweight — fix-inline, no separate review loop. If you find structural issues, ask the operator before proceeding to issue decomposition.
```

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-define/SKILL.md
git commit -m "feat(autospec-define): add spec self-review pass (Phase 2.5)"
```

### Task B2: Add shell-structure decomposer questions

**Files:**
- Modify: `skills/autospec-define/SKILL.md`

- [ ] **Step 1: Locate the decomposition phase**

In `skills/autospec-define/SKILL.md`, find the section that describes how the LLM decides what each issue should contain.

- [ ] **Step 2: Insert shell-structure questions**

In the decomposition phase, add:

```markdown
For each candidate issue, the decomposer asks itself three questions before drafting the issue body. These shape decomposition quality and are NOT written into the issue body as YAML:

- **Produces**: What new files, new exports, or new behavior does this issue create? If the answer is "edits scattered across many existing files with no clear new contract," reconsider the boundary.
- **Consumes**: What existing files or outputs of earlier issues does this depend on? Each named dependency on an earlier issue translates to a lock-step label on this issue.
- **Covers**: Which sections of the spec does this issue implement? If multiple unrelated sections, split. If no spec section, reconsider whether the issue belongs.

If two adjacent issues have heavy mutual Consumes/Produces overlap, they probably want to be merged. If one issue has more than ~5 named Produces, it probably wants to be split.
```

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-define/SKILL.md
git commit -m "feat(autospec-define): decomposer asks Produces/Consumes/Covers questions"
```

### Task B3: Add autospec:v2-flow label to filed issues

**Files:**
- Modify: `skills/autospec-define/SKILL.md`

- [ ] **Step 1: Locate the gh issue create section**

In `skills/autospec-define/SKILL.md`, find where issues are filed via `gh issue create`.

- [ ] **Step 2: Add the v2-flow label to all new issues**

Modify the `gh issue create` invocation guidance to include `--label autospec:v2-flow` on every issue this skill files. Document in the same section:

```markdown
Every issue filed by /autospec-define carries the `autospec:v2-flow` label. The Phase 4 monitor uses this label to route the issue to the absorbed-discipline implementer prompt (expand → implement → finalize → peer-review → evaluate). Issues filed by older autospec versions lack this label and route to the legacy implementer path.

If the repo does not yet have the `autospec:v2-flow` label defined, create it once with:

    gh label create autospec:v2-flow --description "Routes to absorbed-discipline Phase 4 implementer" --color 0e8a16
```

Add a Phase 3 step that ensures the label exists before issue creation (idempotent `gh label create` with `|| true` to ignore "already exists" errors).

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-define/SKILL.md
git commit -m "feat(autospec-define): label new issues autospec:v2-flow for Phase 4 routing"
```

**Phase B ship checkpoint**: open PR. New issues now carry the v2-flow label and pass through a self-review pass. Phase 4 implementer change in Phase C lets the label actually matter.

---

## Phase C — Phase 4 implementer prompt with absorbed turbo discipline

Goal of phase: the autonomous Phase 4 monitor subagent, when it picks up an issue with `autospec:v2-flow`, loads a self-contained prompt that walks the expand → implement → finalize → peer-review → evaluate-findings discipline inline. No skill tool calls inside the subagent.

### Task C1: Write the Phase 4 implementer prompt template

**Files:**
- Create: `skills/autospec-run/prompts/phase4-implementer.md`
- Test: `tests/phase4/test_prompt_structure.sh`

- [ ] **Step 1: Write the failing test**

Create `tests/phase4/test_prompt_structure.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
prompt="$SCRIPT_DIR/skills/autospec-run/prompts/phase4-implementer.md"

[[ -f "$prompt" ]] || { echo "FAIL: prompt file missing"; exit 1; }

for section in "## Expand" "## Implement" "## Finalize" "## Peer-review" "## Evaluate findings" "## Lock-step compliance"; do
    grep -qF "$section" "$prompt" || { echo "FAIL: section missing: $section"; exit 1; }
done

# Must reference graceful Codex skip
grep -qi "codex.*not.*path\|codex.*absent\|gracefully skip" "$prompt" || { echo "FAIL: graceful Codex skip not documented"; exit 1; }

# Must NOT invoke Skill tool inside (self-contained)
! grep -qE 'invoke .* skill|use the Skill tool|Skill\(' "$prompt" || { echo "FAIL: prompt references Skill invocations (must be self-contained)"; exit 1; }

echo "PASS"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash tests/phase4/test_prompt_structure.sh`
Expected: FAIL with "prompt file missing"

- [ ] **Step 3: Write the prompt template**

Create `skills/autospec-run/prompts/phase4-implementer.md`:

```markdown
# Phase 4 implementer prompt (autospec:v2-flow)

You are the autospec Phase 4 implementer subagent. You have been handed one GitHub issue with the `autospec:v2-flow` label. Your job is to take it from "open" to "PR merged" without operator intervention, following the steps below in order. Do not invoke any Skill tool from within this subagent — every instruction you need is here.

## Inputs

- Issue number: provided by the monitor.
- Issue body: read with `gh issue view <N> --json title,body,labels`.
- Tier labels: `ctx:*` and `reasoning:*` set your context budget and reasoning depth (see autospec model-tier rules).
- Lock-step deps: any `lockstep-after:<N>` label means issue N must be merged before you may open your PR. Re-check this immediately before opening the PR.

## Expand

Before changing any code:

1. Read the issue body in full.
2. Verify that every file path the issue references actually exists at the cited path. If a referenced file was renamed or removed, do NOT guess — post a comment on the issue describing the mismatch and exit with the issue reassigned for human review.
3. Run a quick pattern survey for analogous existing implementations: `grep -r <key term> --include="*.<ext>"` and `find . -name "<pattern>"`. The goal is to identify the existing conventions you should follow, not to produce a survey artifact.
4. If the issue's contract is ambiguous in a way that affects implementation (two valid interpretations, missing acceptance criteria), post a clarifying comment on the issue and exit. Do not guess.

## Implement

1. Stay within the context and reasoning budget implied by the issue's `ctx:*` / `reasoning:*` labels. If you hit budget pressure, stop and post a comment on the issue rather than producing rushed work.
2. Follow the conventions surfaced during Expand. Do not introduce new patterns that diverge from surrounding code unless the issue explicitly asks for them.
3. Write tests first when the change has a clear functional contract. For pure prose / docs changes, skip TDD.
4. Commit in small, conventional commits as you go. Final PR can squash; intermediate commits are for your own checkpointing.

## Finalize

Before considering the work done:

1. Run the project's test command (consult the repo's CI config or AGENTS.md). All tests must pass.
2. Run the project's lint/format command. Fix or `git stash` any unrelated noise — do not include unrelated cleanups.
3. Verify the diff matches the issue's scope. If you ended up touching more than the issue called for, either split the extra work into a separate issue or revert it from this branch.
4. Commit message follows the repo's existing style (see recent `git log --oneline`).

## Peer-review

If the `codex` CLI is on PATH, get a second opinion on the diff:

    git diff main...HEAD | codex exec --prompt "Review this diff for correctness, security, broken tests, and consistency with surrounding code. For each finding, label it must-fix or nice-to-have. Be brief."

If `codex` is NOT on PATH: skip this step entirely, log a single line `Peer-review: codex not on PATH, skipping` in the eventual PR description, and proceed.

Capture the Codex output to `.autospec/peer-review-<issue-N>.txt` (gitignored).

## Evaluate findings

If Peer-review ran:

1. Parse the Codex output. Separate findings into:
   - **Must-fix**: correctness bugs, security issues, broken tests, clearly wrong code.
   - **Nice-to-have**: style preferences, scope creep, opinions, alternative designs.
2. Apply must-fix findings as additional commits on this branch. Re-run tests after.
3. Append the nice-to-have findings verbatim to the PR description under a `## Peer-review notes (not addressed)` heading, so the human reviewer can decide.
4. If Codex output is empty or just "looks good", note that too.

If Peer-review skipped: skip this step.

## Lock-step compliance

Immediately before `gh pr create`:

1. Re-check every `lockstep-after:<N>` label on the issue. For each, run `gh pr view <N> --json state,mergedAt`. All must show `state: MERGED` with a non-null `mergedAt`.
2. If any lockstep dep is not yet merged: do NOT open the PR. Comment on the issue noting which dep is blocking, and exit. The monitor will pick this issue up again later.
3. If all deps are merged: open the PR with `gh pr create`. PR body must include "Closes #<issue-N>".

## Exit conditions

- Success: PR opened, all checks green, auto-merge enabled.
- Soft fail (return to queue): clarification needed, lockstep blocked, budget exhausted. Comment on the issue; do not open a PR.
- Hard fail (escalate): test infrastructure broken, repo in inconsistent state, conflicting changes detected. Comment on the issue with `escalate:human` label.
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash tests/phase4/test_prompt_structure.sh`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add skills/autospec-run/prompts/phase4-implementer.md tests/phase4/test_prompt_structure.sh
git commit -m "feat(autospec-run): Phase 4 implementer prompt with absorbed turbo discipline"
```

### Task C2: Route autospec:v2-flow issues to the new prompt

**Files:**
- Modify: `skills/autospec-run/SKILL.md`

- [ ] **Step 1: Locate the Phase 4 monitor / implementer dispatch in SKILL.md**

Run: `grep -n "Phase 4\|implementer" skills/autospec-run/SKILL.md`

- [ ] **Step 2: Insert label-based routing**

In the Phase 4 implementer dispatch section, add:

```markdown
### Implementer prompt selection

Before dispatching the implementer subagent, read the issue's labels:

    labels=$(gh issue view <N> --json labels --jq '[.labels[].name] | join(",")')

If labels contain `autospec:v2-flow`: load the subagent prompt from `skills/autospec-run/prompts/phase4-implementer.md` (absorbed-discipline path: expand → implement → finalize → peer-review → evaluate-findings).

Otherwise: use the legacy implementer prompt (current behavior). Legacy path is retained until all pre-v2 issues drain.

Both paths share lock-step compliance and the same monitor outer loop.
```

- [ ] **Step 3: Commit**

```bash
git add skills/autospec-run/SKILL.md
git commit -m "feat(autospec-run): route autospec:v2-flow issues to absorbed-discipline prompt"
```

### Task C3: Document the integration end-to-end in SKILLS.md

**Files:**
- Modify: `SKILLS.md`

- [ ] **Step 1: Add turbo integration paragraph**

In `SKILLS.md`, find the autospec-run entry and add a paragraph after its existing description:

```markdown
**Turbo integration** (since 2026-05-17): Issues filed with the `autospec:v2-flow` label route to a Phase 4 implementer prompt that absorbs turbo's expand → implement → finalize → peer-review → evaluate-findings discipline inline. Peer-review uses Codex CLI; gracefully skips when absent. See `docs/superpowers/specs/2026-05-17-turbo-autospec-integration-design.md`.
```

- [ ] **Step 2: Commit**

```bash
git add SKILLS.md
git commit -m "docs(skills): note turbo integration on autospec-run"
```

**Phase C ship checkpoint**: open PR. After merge, the next time `/autospec-define` files issues, they'll route through the absorbed-discipline implementer.

---

## Self-Review

**Spec coverage check:**
- Spec §"Planning side: concept absorption inside /autospec-define" → Tasks B1 (self-review pass), B2 (shell-structure questions), B3 (v2-flow label). ✓
- Spec §"Implementation side: Phase 4 implementer prompt" → Tasks C1 (prompt file), C2 (routing). ✓
- Spec §"Install / update — single entrypoint" → Tasks A1-A6 (helpers, bootstrap, codex check, CLAUDE.md merge, gitignore, --update). Task A7 (README). ✓
- Spec §"Detection: how Phase 4 routes issues" → Task B3 (apply label) + Task C2 (read label). ✓
- Spec §"Concept mapping" → documented in Task A4 CLAUDE.md block + Task C3 SKILLS.md note. ✓
- Spec §"Risks and mitigations" → graceful Codex skip implemented in Task A3 (install check) + Task C1 (prompt graceful skip). ✓

**Placeholder scan:** No "TBD", "TODO" in plan body. Each step has concrete content.

**Type / name consistency:**
- Marker name `autospec-block` consistent across Tasks A1, A4.
- Label name `autospec:v2-flow` consistent across Tasks B3, C1, C2.
- Path `skills/autospec-run/prompts/phase4-implementer.md` consistent across Tasks C1, C2.
- Function names `merge_marked_block`, `bootstrap_turbo`, `check_codex`, `merge_claude_md`, `offer_gitignore`, `update_autospec` consistent within `install.sh`.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-17-turbo-autospec-integration.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Each of the three phases ends in a ship checkpoint where you can review/merge before the next phase starts.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
