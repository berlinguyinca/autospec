#!/usr/bin/env bats
# tests/unit/test_autospec_design.bats — scaffold-level tests for the
# autospec-design skill: presence of the 6 lockstep trio + install/uninstall
# + README files, lockstep equality (SKILL body == opencode body == codex
# body), and round-trip install/uninstall behavior across all three harnesses.
#
# Spec ref: docs/specs/2026-05-26-autospec-design-skill.md § Skill layout.
# Mirrors tests/unit/test_install_listener.bats.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SKILL_DIR="$REPO_ROOT/skills/autospec-design"
    INSTALL="$SKILL_DIR/install.sh"
    UNINSTALL="$SKILL_DIR/uninstall.sh"

    # Per-test fake HOME — so the real user environment never sees our writes
    # and tests run hermetically.
    FAKE_HOME="$(mktemp -d)"
    export HOME="$FAKE_HOME"
    export CLAUDE_CONFIG_DIR="$FAKE_HOME/.claude"
    export OPENCODE_CONFIG_DIR="$FAKE_HOME/.config/opencode"
    export CODEX_HOME="$FAKE_HOME/.codex"
    export AUTOSPEC_NO_SELF_UPDATE=1

    # Expected destinations (mirror install.sh).
    CLAUDE_DEST="$CLAUDE_CONFIG_DIR/skills/autospec-design/SKILL.md"
    OPENCODE_DEST="$OPENCODE_CONFIG_DIR/agent/autospec-design.md"
    CODEX_DEST="$CODEX_HOME/prompts/autospec-design.md"
}

teardown() {
    if [ -n "${FAKE_HOME:-}" ] && [ -d "$FAKE_HOME" ]; then
        rm -rf "$FAKE_HOME"
    fi
}

# ---- 6-file scaffold presence --------------------------------------------

@test "scaffold: SKILL.md exists" {
    [ -f "$SKILL_DIR/SKILL.md" ]
}

@test "scaffold: opencode/agent.md exists" {
    [ -f "$SKILL_DIR/opencode/agent.md" ]
}

@test "scaffold: codex/prompt.md exists" {
    [ -f "$SKILL_DIR/codex/prompt.md" ]
}

@test "scaffold: install.sh exists and is executable" {
    [ -f "$SKILL_DIR/install.sh" ]
    [ -x "$SKILL_DIR/install.sh" ]
}

@test "scaffold: uninstall.sh exists and is executable" {
    [ -f "$SKILL_DIR/uninstall.sh" ]
    [ -x "$SKILL_DIR/uninstall.sh" ]
}

@test "scaffold: README.md exists" {
    [ -f "$SKILL_DIR/README.md" ]
}

# ---- lockstep: SKILL body == opencode body == codex body -----------------

@test "lockstep: SKILL.md body (post-frontmatter) == codex/prompt.md verbatim" {
    skill_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md")"
    codex_body="$(cat "$SKILL_DIR/codex/prompt.md")"
    [ "$skill_body" = "$codex_body" ]
}

@test "lockstep: SKILL.md body (post-frontmatter) == opencode body (post-frontmatter)" {
    skill_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md")"
    opencode_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/opencode/agent.md")"
    [ "$skill_body" = "$opencode_body" ]
}

# ---- install --dry-run --harness all ---------------------------------------

@test "install --dry-run --harness all lists all destinations and exits 0" {
    run sh "$INSTALL" --harness all --dry-run
    [ "$status" -eq 0 ]
    echo "$output" | grep -q "autospec-design"
}

# ---- install creates expected files --------------------------------------

@test "install creates expected files for --harness all" {
    run sh "$INSTALL" --harness all
    [ "$status" -eq 0 ]
    [ -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]
}

@test "install --harness claude only places the Claude file" {
    run sh "$INSTALL" --harness claude
    [ "$status" -eq 0 ]
    [ -f "$CLAUDE_DEST" ]
    [ ! -f "$OPENCODE_DEST" ]
    [ ! -f "$CODEX_DEST" ]
}

@test "install --harness opencode only places the OpenCode file" {
    run sh "$INSTALL" --harness opencode
    [ "$status" -eq 0 ]
    [ ! -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ ! -f "$CODEX_DEST" ]
}

@test "install --harness codex only places the Codex prompt" {
    run sh "$INSTALL" --harness codex
    [ "$status" -eq 0 ]
    [ ! -f "$CLAUDE_DEST" ]
    [ ! -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]
}

# ---- round-trip install + uninstall --------------------------------------

@test "uninstall removes files placed by install (--harness all)" {
    sh "$INSTALL" --harness all >/dev/null
    [ -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]

    run sh "$UNINSTALL" --harness all
    [ "$status" -eq 0 ]
    [ ! -f "$CLAUDE_DEST" ]
    [ ! -f "$OPENCODE_DEST" ]
    [ ! -f "$CODEX_DEST" ]
}

# ---- idempotency ---------------------------------------------------------

@test "install is idempotent (running twice in a row exits 0 both times)" {
    run sh "$INSTALL" --harness all
    [ "$status" -eq 0 ]
    run sh "$INSTALL" --harness all
    [ "$status" -eq 0 ]
    [ -f "$CLAUDE_DEST" ]
    [ -f "$OPENCODE_DEST" ]
    [ -f "$CODEX_DEST" ]
}

@test "uninstall is idempotent (running twice in a row exits 0 both times)" {
    sh "$INSTALL" --harness all >/dev/null
    run sh "$UNINSTALL" --harness all
    [ "$status" -eq 0 ]
    run sh "$UNINSTALL" --harness all
    [ "$status" -eq 0 ]
}

# ---- validate.sh wiring (issue #573) -------------------------------------

@test "validate.sh: check_startup_preflight enumerates autospec-design" {
    sed -n '/^check_startup_preflight()/,/^}/p' "$REPO_ROOT/autospec validate" \
        | grep -q 'autospec-design'
}

@test "validate.sh: check_codex_skills_install enumerates autospec-design" {
    sed -n '/^check_codex_skills_install()/,/^}/p' "$REPO_ROOT/autospec validate" \
        | grep -q 'autospec-design'
}

@test "validate.sh: check_shared_script_install enumerates autospec-design" {
    sed -n '/^check_shared_script_install()/,/^}/p' "$REPO_ROOT/autospec validate" \
        | grep -q 'autospec-design'
}

@test "validate.sh: check_subagent_model_tier has autospec-design case branch with expected_a=2 expected_b=0" {
    block="$(sed -n '/^check_subagent_model_tier()/,/^}/p' "$REPO_ROOT/autospec validate")"
    # Capture from "autospec-design)" up to and including the ";;" terminator.
    case_body="$(printf '%s\n' "$block" \
        | awk '/autospec-design)/{f=1} f{print; if (/;;/) exit}')"
    [ -n "$case_body" ]
    printf '%s\n' "$case_body" | grep -q 'expected_a=2'
    printf '%s\n' "$case_body" | grep -q 'expected_b=0'
}

@test "validate.sh: autospec-design appears in at least 4 hardcoded skill enumerations" {
    count="$(grep -c 'autospec-design' "$REPO_ROOT/autospec validate")"
    [ "$count" -ge 4 ]
}

@test "validate.sh: autospec validate exits 0 against current tree" {
    run bash "$REPO_ROOT/autospec validate"
    [ "$status" -eq 0 ]
}

# ---- top-level install.sh wiring (issue #574) ----------------------------

@test "top-level install.sh: auto-discovery includes autospec-design" {
    # install.sh builds ALL_SKILLS dynamically from skills/*/ that ship an
    # install.sh (#705 replaced the old static list + case statement so newly
    # added skills are never silently missed). The discovery condition for
    # autospec-design is therefore that it ships an install.sh.
    [ -f "$REPO_ROOT/skills/autospec-design/install.sh" ]
    # And it must actually land in the runtime-discovered ALL_SKILLS.
    discovered="$(for d in "$REPO_ROOT"/skills/*/; do \
        [ -f "$d/install.sh" ] && basename "$d"; done)"
    printf '%s\n' "$discovered" | grep -qx 'autospec-design'
}

@test "top-level install.sh: --skill validation accepts autospec-design (rejects bogus)" {
    # The validator loops the dynamically-discovered ALL_SKILLS (no hardcoded
    # case), so it accepts autospec-design and rejects an unknown skill.
    run env AUTOSPEC_NO_STAR_PROMPT=1 AUTOSPEC_NO_SELF_UPDATE=1 \
        bash "$REPO_ROOT/install.sh" --skill autospec-design --harness all --dry-run
    [ "$status" -eq 0 ]
    run env AUTOSPEC_NO_STAR_PROMPT=1 AUTOSPEC_NO_SELF_UPDATE=1 \
        bash "$REPO_ROOT/install.sh" --skill autospec-bogus-xyz --harness all --dry-run
    [ "$status" -ne 0 ]
    printf '%s\n' "$output" | grep -q 'invalid --skill'
}

@test "install.sh --skill autospec-design --harness all --dry-run exits 0" {
    run env AUTOSPEC_NO_STAR_PROMPT=1 AUTOSPEC_NO_SELF_UPDATE=1 \
        bash "$REPO_ROOT/install.sh" --skill autospec-design --harness all --dry-run
    [ "$status" -eq 0 ]
}

@test "install.sh --skill bogus-skill --dry-run exits non-zero (regression guard)" {
    run env AUTOSPEC_NO_STAR_PROMPT=1 AUTOSPEC_NO_SELF_UPDATE=1 \
        bash "$REPO_ROOT/install.sh" --skill bogus-skill --dry-run
    [ "$status" -ne 0 ]
}

# ---- fetch-design-md: catalog fetcher (issue #576) -----------------------
# Tests cover cache-hit, missing-vendor Levenshtein hints, missing-args, and
# missing-tools failure modes. Network-touching cases use the cache short-
# circuit and an isolated $HOME so they run hermetically without hitting
# the real catalog. The script reads $AUTOSPEC_DESIGN_CACHE_DIR and
# $AUTOSPEC_DESIGN_CACHE_TTL to make these paths configurable.

setup_fetch() {
    FETCH="$SKILL_DIR/scripts/fetch-design-md.sh"
    export AUTOSPEC_DESIGN_CACHE_DIR="$FAKE_HOME/.autospec/design-cache"
    export AUTOSPEC_DESIGN_CACHE_TTL=86400
}

@test "fetch-design-md: script exists and is executable" {
    setup_fetch
    [ -f "$FETCH" ]
    [ -x "$FETCH" ]
}

@test "fetch-design-md: no args exits non-zero with usage message" {
    setup_fetch
    run bash "$FETCH"
    [ "$status" -ne 0 ]
    echo "$output" | grep -qi 'usage'
}

@test "fetch-design-md: fresh cache hit prints body without network" {
    setup_fetch
    vendor="testvendor"
    cache_file="$AUTOSPEC_DESIGN_CACHE_DIR/$vendor/DESIGN.md"
    mkdir -p "$(dirname "$cache_file")"
    printf '# Test DESIGN.md\n\nHello from cache.\n' > "$cache_file"
    # Make PATH empty so any accidental network call would fail loudly.
    run env PATH="/usr/bin:/bin" bash "$FETCH" "$vendor"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q 'Hello from cache'
}

@test "fetch-design-md: stale cache (mtime > TTL) triggers re-fetch attempt" {
    setup_fetch
    vendor="testvendor-stale"
    cache_file="$AUTOSPEC_DESIGN_CACHE_DIR/$vendor/DESIGN.md"
    mkdir -p "$(dirname "$cache_file")"
    printf 'stale body\n' > "$cache_file"
    # Force the file to look ancient (1970-ish).
    touch -t 197001010000 "$cache_file" 2>/dev/null || touch -d '1970-01-01' "$cache_file"
    # Set TTL low (1s) and point catalog at an unreachable host so the
    # re-fetch fails loudly rather than silently using stale data.
    export AUTOSPEC_DESIGN_CACHE_TTL=1
    export AUTOSPEC_DESIGN_CATALOG_OWNER="autospec-test-nonexistent-owner-zzz"
    export AUTOSPEC_DESIGN_CATALOG_REPO="nonexistent-repo-zzz"
    run env PATH="/usr/bin:/bin" bash "$FETCH" "$vendor"
    # Must NOT silently print stale body — must exit non-zero on network failure.
    [ "$status" -ne 0 ]
}

@test "fetch-design-md: missing both gh and curl exits non-zero with install hint" {
    setup_fetch
    vendor="anyvendor"
    # Stub PATH containing only shims for the binaries the script needs
    # internally (date, stat, cat, mkdir, printf, dirname, sort, head,
    # awk, sed, grep, tr) but NOT gh and NOT curl. We symlink from the
    # real locations into a temp dir.
    stub_path="$FAKE_HOME/stub-bin"
    mkdir -p "$stub_path"
    for tool in bash sh date stat cat mkdir printf dirname sort head awk sed grep tr base64 seq; do
        src="$(command -v "$tool" 2>/dev/null || true)"
        [ -n "$src" ] && ln -sf "$src" "$stub_path/$tool"
    done
    run env -i HOME="$FAKE_HOME" \
        AUTOSPEC_DESIGN_CACHE_DIR="$AUTOSPEC_DESIGN_CACHE_DIR" \
        AUTOSPEC_DESIGN_CACHE_TTL="$AUTOSPEC_DESIGN_CACHE_TTL" \
        PATH="$stub_path" bash "$FETCH" "$vendor"
    [ "$status" -ne 0 ]
    echo "$output" | grep -qiE 'install|gh|curl'
}

# ---- score-suggestion: rubric scorer (issue #577) ------------------------
# Tests build throwaway fixture repos under $BATS_TEST_TMPDIR and inject the
# catalog vendor list via $AUTOSPEC_DESIGN_VENDORS so the scorer runs
# hermetically without hitting the real catalog. The vendor list mirrors a
# representative slice of berlinguyinca/awesome-design-md.

SCORE_VENDORS="linear.app vercel stripe apple tesla notion figma cursor supabase nike"

setup_score() {
    SCORE="$SKILL_DIR/scripts/score-suggestion.sh"
    NEXT_REPO="$BATS_TEST_TMPDIR/nextjs-app"
    HTML_REPO="$BATS_TEST_TMPDIR/vanilla-html"

    mkdir -p "$NEXT_REPO"
    cat > "$NEXT_REPO/package.json" <<'EOF'
{
  "name": "linear-style-dashboard",
  "description": "A developer-tool dashboard inspired by Linear's design language.",
  "dependencies": { "next": "14.0.0", "react": "18.2.0" }
}
EOF
    cat > "$NEXT_REPO/next.config.js" <<'EOF'
module.exports = {};
EOF
    cat > "$NEXT_REPO/README.md" <<'EOF'
# Linear-style developer dashboard

A Next.js app for engineering teams, modeled after Linear.
EOF

    mkdir -p "$HTML_REPO"
    cat > "$HTML_REPO/index.html" <<'EOF'
<!doctype html>
<html><head><title>Tesla showroom</title></head>
<body><h1>Electric vehicles by Tesla</h1></body></html>
EOF
    cat > "$HTML_REPO/README.md" <<'EOF'
# Tesla electric vehicle marketing site

A static HTML brochure site for Tesla cars.
EOF
}

@test "score-suggestion: script exists and is executable" {
    setup_score
    [ -f "$SCORE" ]
    [ -x "$SCORE" ]
}

# linter:allow-COMPLEXITY legacy test module remains intentionally consolidated
@test "score-suggestion: script contains no standalone any token" {
    setup_score
    run grep -nE '(^|[^[:alnum:]_])any([^[:alnum:]_]|$)' "$SCORE"
    [ "$status" -ne 0 ]
}

@test "score-suggestion: no repo arg exits non-zero with usage" {
    setup_score
    run bash "$SCORE"
    [ "$status" -ne 0 ]
    echo "$output" | grep -qi 'usage'
}

@test "score-suggestion: Next.js fixture scores linear at least 3" {
    setup_score
    run env AUTOSPEC_DESIGN_VENDORS="$SCORE_VENDORS" bash "$SCORE" "$NEXT_REPO"
    [ "$status" -eq 0 ]
    # Output lines look like: "<score>\t<vendor>\t<rationale>". Find linear's score.
    linear_line="$(echo "$output" | grep -iE 'linear' | head -1)"
    [ -n "$linear_line" ]
    linear_score="$(echo "$linear_line" | grep -oE '^[0-9]+' | head -1)"
    [ -n "$linear_score" ]
    [ "$linear_score" -ge 3 ]
}

@test "score-suggestion: prints at most 3 vendors" {
    setup_score
    run env AUTOSPEC_DESIGN_VENDORS="$SCORE_VENDORS" bash "$SCORE" "$NEXT_REPO"
    [ "$status" -eq 0 ]
    # Count vendor result lines (those beginning with a score integer).
    n="$(echo "$output" | grep -cE '^[0-9]+')"
    [ "$n" -le 3 ]
    [ "$n" -ge 1 ]
}

@test "score-suggestion: vanilla HTML top-1 differs from Next.js top-1" {
    setup_score
    run env AUTOSPEC_DESIGN_VENDORS="$SCORE_VENDORS" bash "$SCORE" "$NEXT_REPO"
    [ "$status" -eq 0 ]
    next_top="$(echo "$output" | grep -E '^[0-9]+' | head -1 | cut -f2)"

    run env AUTOSPEC_DESIGN_VENDORS="$SCORE_VENDORS" bash "$SCORE" "$HTML_REPO"
    [ "$status" -eq 0 ]
    html_top="$(echo "$output" | grep -E '^[0-9]+' | head -1 | cut -f2)"

    [ -n "$next_top" ]
    [ -n "$html_top" ]
    [ "$next_top" != "$html_top" ]
}

# ---- suggest-section trio prose + lockstep (issue #577) ------------------

@test "suggest: SKILL.md has a '## Suggest' heading" {
    run grep -c '^## Suggest' "$SKILL_DIR/SKILL.md"
    [ "$status" -eq 0 ]
    [ "$output" -eq 1 ]
}

@test "suggest: lockstep holds after prose addition (SKILL == codex)" {
    skill_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md")"
    codex_body="$(cat "$SKILL_DIR/codex/prompt.md")"
    [ "$skill_body" = "$codex_body" ]
}

@test "suggest: lockstep holds after prose addition (SKILL == opencode)" {
    skill_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md")"
    opencode_body="$(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/opencode/agent.md")"
    [ "$skill_body" = "$opencode_body" ]
}

# ---- apply-section trio prose + lockstep + runnable flow (issue #578) -----
# The apply subcommand is prose-driven (no standalone helper, per #578 scope):
# the SKILL.md ## Apply section embeds a self-contained, copy-pasteable bash
# flow inside a ```bash fenced block tagged with the marker comment
# "# autospec-design-apply-flow". These tests extract that exact flow and run
# it against a REAL git fixture repo (no mocks) to prove the documented
# commands actually write DESIGN.md, create the feature branch, and produce
# the spec'd commit message. The fetch step is satisfied via the cache short-
# circuit (the same mechanism fetch-design-md.sh uses), so no network is hit.

# Extract the fenced apply flow from SKILL.md into $1 (a file path).
extract_apply_flow() {
    awk '
        /# autospec-design-apply-flow/ { capture=1 }
        capture && /^```[[:space:]]*$/ { exit }
        capture { print }
    ' "$SKILL_DIR/SKILL.md" > "$1"
}

setup_apply() {
    APPLY_REPO="$BATS_TEST_TMPDIR/apply-fixture"
    mkdir -p "$APPLY_REPO"
    git -C "$APPLY_REPO" init -q
    git -C "$APPLY_REPO" config user.email "test@example.com"
    git -C "$APPLY_REPO" config user.name "Test User"
    git -C "$APPLY_REPO" commit -q --allow-empty -m "init"
    # Seed the design cache so the flow's fetch is a hermetic cache hit.
    export AUTOSPEC_DESIGN_CACHE_DIR="$FAKE_HOME/.autospec/design-cache"
    export AUTOSPEC_DESIGN_CACHE_TTL=86400
    mkdir -p "$AUTOSPEC_DESIGN_CACHE_DIR/linear.app"
    printf '# Linear design language\n\nBody from cache.\n' \
        > "$AUTOSPEC_DESIGN_CACHE_DIR/linear.app/DESIGN.md"
    FETCH="$SKILL_DIR/scripts/fetch-design-md.sh"
}

@test "apply: SKILL.md has exactly one '## Apply' heading" {
    run grep -c '^## Apply' "$SKILL_DIR/SKILL.md"
    [ "$status" -eq 0 ]
    [ "$output" -eq 1 ]
}

@test "apply: SKILL.md documents the spec'd commit message" {
    grep -q 'feat(design): adopt .* design language' "$SKILL_DIR/SKILL.md"
}

@test "apply: SKILL.md documents the --force refuse-overwrite guard" {
    grep -qi 'force' "$SKILL_DIR/SKILL.md"
    grep -qi 'overwrite' "$SKILL_DIR/SKILL.md"
}

@test "apply: SKILL.md embeds a runnable apply flow block" {
    flow="$BATS_TEST_TMPDIR/flow.sh"
    extract_apply_flow "$flow"
    [ -s "$flow" ]
    run bash -n "$flow"
    [ "$status" -eq 0 ]
}

@test "apply: runnable flow writes DESIGN.md to root in feat/design-<vendor>" {
    setup_apply
    flow="$BATS_TEST_TMPDIR/flow.sh"
    extract_apply_flow "$flow"
    run env DESIGN_VENDOR="linear.app" \
        DESIGN_REPO_ROOT="$APPLY_REPO" \
        DESIGN_FETCH="$FETCH" \
        DESIGN_NO_PUSH=1 \
        bash "$flow"
    [ "$status" -eq 0 ]
    [ -f "$APPLY_REPO/DESIGN.md" ]
    grep -q 'Body from cache' "$APPLY_REPO/DESIGN.md"
    # Branch feat/design-linear.app created and checked out.
    branch="$(git -C "$APPLY_REPO" rev-parse --abbrev-ref HEAD)"
    [ "$branch" = "feat/design-linear.app" ]
    # Commit message matches the spec.
    msg="$(git -C "$APPLY_REPO" log -1 --pretty=%s)"
    [ "$msg" = "feat(design): adopt linear.app design language" ]
}

@test "apply: runnable flow refuses overwrite without --force, exits non-zero" {
    setup_apply
    printf 'PRE-EXISTING\n' > "$APPLY_REPO/DESIGN.md"
    flow="$BATS_TEST_TMPDIR/flow.sh"
    extract_apply_flow "$flow"
    run env DESIGN_VENDOR="linear.app" \
        DESIGN_REPO_ROOT="$APPLY_REPO" \
        DESIGN_FETCH="$FETCH" \
        DESIGN_NO_PUSH=1 \
        bash "$flow"
    [ "$status" -ne 0 ]
    # File unchanged.
    grep -q 'PRE-EXISTING' "$APPLY_REPO/DESIGN.md"
}

@test "apply: runnable flow overwrites with --force" {
    setup_apply
    printf 'PRE-EXISTING\n' > "$APPLY_REPO/DESIGN.md"
    flow="$BATS_TEST_TMPDIR/flow.sh"
    extract_apply_flow "$flow"
    run env DESIGN_VENDOR="linear.app" \
        DESIGN_REPO_ROOT="$APPLY_REPO" \
        DESIGN_FETCH="$FETCH" \
        DESIGN_FORCE=1 \
        DESIGN_NO_PUSH=1 \
        bash "$flow"
    [ "$status" -eq 0 ]
    grep -q 'Body from cache' "$APPLY_REPO/DESIGN.md"
    ! grep -q 'PRE-EXISTING' "$APPLY_REPO/DESIGN.md"
}

@test "apply: lockstep holds after prose addition (SKILL == codex)" {
    run diff <(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md") \
        "$SKILL_DIR/codex/prompt.md"
    [ "$status" -eq 0 ]
}

@test "apply: lockstep holds after prose addition (SKILL == opencode)" {
    run diff <(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md") \
        <(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/opencode/agent.md")
    [ "$status" -eq 0 ]
}

# ---- migrate helpers: scan UI sources + generate migration spec (#579) -----

setup_migrate_helpers() {
    SCAN="$SKILL_DIR/scripts/scan-ui-sources.sh"
    GEN_MIGRATION="$SKILL_DIR/scripts/gen-migration-spec.sh"
    UI_REPO="$BATS_TEST_TMPDIR/ui-repo"
    EMPTY_REPO="$BATS_TEST_TMPDIR/empty-repo"

    mkdir -p "$UI_REPO/app" "$UI_REPO/components" "$UI_REPO/node_modules/ignored"
    cat > "$UI_REPO/package.json" <<'EOF'
{
  "name": "ui-repo",
  "dependencies": { "next": "14.0.0", "react": "18.2.0" }
}
EOF
    cat > "$UI_REPO/next.config.js" <<'EOF'
module.exports = {};
EOF
    cat > "$UI_REPO/app/page.tsx" <<'EOF'
export default function Page() { return <main>Hello</main>; }
EOF
    cat > "$UI_REPO/components/Button.tsx" <<'EOF'
export function Button() { return <button>Go</button>; }
EOF
    cat > "$UI_REPO/node_modules/ignored/Skip.tsx" <<'EOF'
export const Skip = () => null;
EOF

    mkdir -p "$EMPTY_REPO"

    export AUTOSPEC_DESIGN_CACHE_DIR="$FAKE_HOME/.autospec/design-cache"
    export AUTOSPEC_DESIGN_CACHE_TTL=86400
    mkdir -p "$AUTOSPEC_DESIGN_CACHE_DIR/linear"
    printf '# Linear design language\n\nUse crisp product UI.\n' \
        > "$AUTOSPEC_DESIGN_CACHE_DIR/linear/DESIGN.md"
}

@test "migrate helpers: scan-ui-sources.sh exists and is executable" {
    setup_migrate_helpers
    [ -x "$SCAN" ]
}

@test "migrate helpers: gen-migration-spec.sh exists and is executable" {
    setup_migrate_helpers
    [ -x "$GEN_MIGRATION" ]
}

@test "migrate helpers: scan detects Next.js and UI files" {
    setup_migrate_helpers
    run bash "$SCAN" "$UI_REPO"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '"framework":"next"'
    echo "$output" | grep -q '"app/page.tsx"'
    echo "$output" | grep -q '"components/Button.tsx"'
    ! echo "$output" | grep -q 'node_modules'
}

@test "migrate helpers: scan caps UI inventory at 50 files" {
    setup_migrate_helpers
    for i in $(seq 1 60); do
        printf 'export const C%s = true;\n' "$i" > "$UI_REPO/components/C$i.tsx"
    done
    run bash "$SCAN" "$UI_REPO"
    [ "$status" -eq 0 ]
    file_count="$(printf '%s\n' "$output" | grep -oE '"[^"]+[.](tsx|jsx|ts|js|vue|svelte|html|css|scss)"' | wc -l | tr -d ' ')"
    [ "$file_count" -eq 50 ]
}

@test "migrate helpers: gen-migration-spec writes mandatory sections" {
    setup_migrate_helpers
    run bash "$GEN_MIGRATION" linear "$UI_REPO"
    [ "$status" -eq 0 ]
    spec_path="$(printf '%s\n' "$output" | tail -1)"
    [ -f "$spec_path" ]
    case "$spec_path" in
        "$UI_REPO"/docs/specs/*-design-migration-linear.md) ;;
        *) echo "unexpected spec path: $spec_path"; false ;;
    esac
    grep -q '^## Source' "$spec_path"
    grep -q '^## Target' "$spec_path"
    grep -q '^## Team personality' "$spec_path"
    grep -q '^## Counter-team' "$spec_path"
    grep -q '^## Per-component outline' "$spec_path"
}

@test "migrate helpers: gen-migration-spec exits non-zero for repos with no UI files" {
    setup_migrate_helpers
    run bash "$GEN_MIGRATION" linear "$EMPTY_REPO"
    [ "$status" -ne 0 ]
    echo "$output" | grep -qi 'no UI files'
}

# ---- migrate-section trio prose + runnable flow (issue #580) --------------

extract_migrate_flow() {
    awk '
        /# autospec-design-migrate-flow/ { capture=1 }
        capture && /^```[[:space:]]*$/ { exit }
        capture { print }
    ' "$SKILL_DIR/SKILL.md" > "$1"
}

setup_migrate_flow() {
    MIGRATE_REPO="$BATS_TEST_TMPDIR/migrate-flow-repo"
    mkdir -p "$MIGRATE_REPO/app"
    git -C "$MIGRATE_REPO" init -q
    git -C "$MIGRATE_REPO" config user.email "test@example.com"
    git -C "$MIGRATE_REPO" config user.name "Test User"
    cat > "$MIGRATE_REPO/package.json" <<'EOF'
{"dependencies":{"next":"14.0.0","react":"18.2.0"}}
EOF
    cat > "$MIGRATE_REPO/next.config.js" <<'EOF'
module.exports = {};
EOF
    cat > "$MIGRATE_REPO/app/page.tsx" <<'EOF'
export default function Page() { return <main>Hello</main>; }
EOF
    git -C "$MIGRATE_REPO" add .
    git -C "$MIGRATE_REPO" commit -q -m "init"

    export AUTOSPEC_DESIGN_CACHE_DIR="$FAKE_HOME/.autospec/design-cache"
    export AUTOSPEC_DESIGN_CACHE_TTL=86400
    mkdir -p "$AUTOSPEC_DESIGN_CACHE_DIR/linear"
    printf '# Linear design language\n\nUse crisp product UI.\n' \
        > "$AUTOSPEC_DESIGN_CACHE_DIR/linear/DESIGN.md"
}

@test "migrate: trio files each have exactly one '## Migrate' heading" {
    run grep -c '^## Migrate' "$SKILL_DIR/SKILL.md"
    [ "$status" -eq 0 ]
    [ "$output" -eq 1 ]
    run grep -c '^## Migrate' "$SKILL_DIR/opencode/agent.md"
    [ "$status" -eq 0 ]
    [ "$output" -eq 1 ]
    run grep -c '^## Migrate' "$SKILL_DIR/codex/prompt.md"
    [ "$status" -eq 0 ]
    [ "$output" -eq 1 ]
}

@test "migrate: SKILL.md embeds a runnable migrate flow block" {
    flow="$BATS_TEST_TMPDIR/migrate-flow.sh"
    extract_migrate_flow "$flow"
    [ -s "$flow" ]
    run bash -n "$flow"
    [ "$status" -eq 0 ]
}

@test "migrate: runnable flow requires DESIGN.md from apply first" {
    setup_migrate_flow
    flow="$BATS_TEST_TMPDIR/migrate-flow.sh"
    extract_migrate_flow "$flow"
    run env DESIGN_VENDOR="linear" \
        DESIGN_REPO_ROOT="$MIGRATE_REPO" \
        DESIGN_SCAN="$SKILL_DIR/scripts/scan-ui-sources.sh" \
        DESIGN_GENERATE="$SKILL_DIR/scripts/gen-migration-spec.sh" \
        bash "$flow"
    [ "$status" -ne 0 ]
    echo "$output" | grep -q 'Run /autospec-design apply linear first'
}

@test "migrate: runnable flow writes and commits migration spec" {
    setup_migrate_flow
    printf '# Linear design language\n\nUse crisp product UI.\n' > "$MIGRATE_REPO/DESIGN.md"
    git -C "$MIGRATE_REPO" add DESIGN.md
    git -C "$MIGRATE_REPO" commit -q -m "feat(design): adopt linear design language"
    flow="$BATS_TEST_TMPDIR/migrate-flow.sh"
    extract_migrate_flow "$flow"
    run env DESIGN_VENDOR="linear" \
        DESIGN_REPO_ROOT="$MIGRATE_REPO" \
        DESIGN_SCAN="$SKILL_DIR/scripts/scan-ui-sources.sh" \
        DESIGN_GENERATE="$SKILL_DIR/scripts/gen-migration-spec.sh" \
        bash "$flow"
    [ "$status" -eq 0 ]
    echo "$output" | grep -q '/autospec-define '
    spec_path="$(printf '%s\n' "$output" | sed -n 's#^autospec-design migrate: wrote \(.*\)\.$#\1#p')"
    [ -f "$spec_path" ]
    grep -q '^## Per-component outline' "$spec_path"
    msg="$(git -C "$MIGRATE_REPO" log -1 --pretty=%s)"
    [ "$msg" = "docs(design): add linear migration spec" ]
}

@test "migrate: lockstep holds after prose addition (SKILL == codex)" {
    run diff <(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md") \
        "$SKILL_DIR/codex/prompt.md"
    [ "$status" -eq 0 ]
}

@test "migrate: lockstep holds after prose addition (SKILL == opencode)" {
    run diff <(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/SKILL.md") \
        <(awk '/^---$/{c++; next} c>=2' "$SKILL_DIR/opencode/agent.md")
    [ "$status" -eq 0 ]
}
