#!/usr/bin/env bats
# Contract tests for issue #1537 proactive security/compliance workstream.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/security-workstream.sh"
    WORK="$(mktemp -d -t security-workstream.XXXXXX)"
}

teardown() {
    [ -d "${WORK:-}" ] && rm -rf "$WORK"
}

@test "scan: ranks SAST, secret, unsafe, and CVE findings by exploitability and exposure" {
    cat > "$WORK/raw.jsonl" <<'JSONL'
{"gap_id":"G1","dimension":"secrets","severity":"must-fix","file":"app/config.env","line":3,"title":"Hardcoded API token","body":"Remove and rotate credential","dedupe_key":"sec-secret"}
{"gap_id":"G2","dimension":"cve","severity":"nice-to-have","file":"Cargo.lock","line":0,"title":"CVE-2026-0001 in transitive package","body":"Upgrade when available","dedupe_key":"sec-cve"}
JSONL
    mkdir -p "$WORK/src"
    cat > "$WORK/src/main.rs" <<'RS'
fn main() { unsafe { std::ptr::read(0 as *const i32); } }
RS

    run bash "$SCRIPT" rank --findings "$WORK/raw.jsonl" --root "$WORK" --out "$WORK/ranked.jsonl"
    [ "$status" -eq 0 ]
    [ -f "$WORK/ranked.jsonl" ]
    first_priority="$(head -n1 "$WORK/ranked.jsonl" | jq -r '.priority')"
    [ "$first_priority" = "P0" ]
    grep -q '"dimension":"unsafe"' "$WORK/ranked.jsonl"
    grep -q '"severity_rank"' "$WORK/ranked.jsonl"
}

@test "rank: detects Rust unsafe syntax without flagging prose or command payloads" {
    : > "$WORK/raw.jsonl"
    mkdir -p "$WORK/src"
    printf '%s\n' 'const NOTE: &str = "unsafe operations require review";' > "$WORK/src/prose.rs"
    printf '%s\n' 'let command = "echo unsafe";' > "$WORK/src/command.rs"
    printf '%s\n' 'let payload = "unsafe {";' > "$WORK/src/string-payload.rs"
    printf '%s\n' 'const RAW: &str = r#"unsafe {"#;' > "$WORK/src/raw-string-payload.rs"
    printf '%s\n' '// unsafe fn documentation only' > "$WORK/src/comment.rs"
    printf '%s\n' 'fn read(ptr: *const i32) -> i32 { unsafe { std::ptr::read(ptr) } }' > "$WORK/src/unsafe.rs"
    printf '%s\n' 'unsafe' '{' '    external_call();' '}' > "$WORK/src/multiline.rs"
    printf '%s\n' 'unsafe /* FFI boundary */ {' '    external_call();' '}' > "$WORK/src/comment-separated.rs"
    printf '%s\n' '#[unsafe(no_mangle)]' 'pub extern "C" fn exported() {}' > "$WORK/src/unsafe-attribute.rs"
    printf '%s\n' '// autospec:unsafe-reviewed: security review #2004; invariant: the FFI boundary accepts only a fixed signal number.' 'unsafe { external_call(); }' > "$WORK/src/reviewed.rs"
    printf '%s\n' '// autospec:unsafe-reviewed: security review #2004' 'unsafe { external_call(); }' > "$WORK/src/missing-invariant.rs"
    printf '%s\n' '// autospec:unsafe-reviewed: security review #2004; invariant: arbitrary code is safe.' 'unsafe { reviewed(); } unsafe { unreviewed(); }' > "$WORK/src/multiple.rs"
    mkdir -p "$WORK/crates/autospec-cli/src/commands/runtime"
    cat > "$WORK/crates/autospec-cli/src/commands/runtime/env.rs" <<'RS'
#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

fn install_signal_handlers() {
    RECEIVED_SIGNAL.store(0, Ordering::Relaxed);
    // SAFETY: SIGINT/SIGTERM are fixed and the handler only writes an atomic.
    unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }
}
RS

    run bash "$SCRIPT" rank --findings "$WORK/raw.jsonl" --root "$WORK" --out "$WORK/ranked.jsonl"
    [ "$status" -eq 0 ]
    unsafe_count="$(jq -s '[.[] | select(.dimension == "unsafe")] | length' "$WORK/ranked.jsonl")"
    [ "$unsafe_count" -eq 8 ]
    jq -e 'select(.dimension == "unsafe" and .file == "src/unsafe.rs" and .line == 1)' "$WORK/ranked.jsonl" >/dev/null
    jq -e 'select(.dimension == "unsafe" and .file == "src/multiline.rs" and .line == 1)' "$WORK/ranked.jsonl" >/dev/null
    jq -e 'select(.dimension == "unsafe" and .file == "src/comment-separated.rs" and .line == 1)' "$WORK/ranked.jsonl" >/dev/null
    jq -e 'select(.dimension == "unsafe" and .file == "src/unsafe-attribute.rs" and .line == 1)' "$WORK/ranked.jsonl" >/dev/null
    jq -e 'select(.dimension == "unsafe" and .file == "src/missing-invariant.rs" and .line == 2)' "$WORK/ranked.jsonl" >/dev/null
    jq -e 'select(.dimension == "unsafe" and .file == "src/reviewed.rs" and .line == 2)' "$WORK/ranked.jsonl" >/dev/null
    [ "$(jq -s '[.[] | select(.dimension == "unsafe" and .file == "src/multiple.rs")] | length' "$WORK/ranked.jsonl")" -eq 2 ]
    ! jq -e 'select(.dimension == "unsafe" and .file == "crates/autospec-cli/src/commands/runtime/env.rs")' "$WORK/ranked.jsonl" >/dev/null

    cat >> "$WORK/crates/autospec-cli/src/commands/runtime/env.rs" <<'RS'

fn duplicated_signal_registration() {
    unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }
}
RS
    run bash "$SCRIPT" rank --findings "$WORK/raw.jsonl" --root "$WORK" --out "$WORK/duplicated.jsonl"
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.dimension == "unsafe" and .file == "crates/autospec-cli/src/commands/runtime/env.rs")] | length' "$WORK/duplicated.jsonl")" -eq 1 ]

    cat > "$WORK/crates/autospec-cli/src/commands/runtime/env.rs" <<'RS'
const DECLARATION: &str = r#"extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}"#;

#[cfg(unix)]
extern "C" {
    #[link_name = "abort"]
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

fn install_signal_handlers() {
    RECEIVED_SIGNAL.store(0, Ordering::Relaxed);
    unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }
}
RS
    run bash "$SCRIPT" rank --findings "$WORK/raw.jsonl" --root "$WORK" --out "$WORK/spoofed.jsonl"
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.dimension == "unsafe" and .file == "crates/autospec-cli/src/commands/runtime/env.rs")] | length' "$WORK/spoofed.jsonl")" -eq 1 ]

    cat > "$WORK/crates/autospec-cli/src/commands/runtime/env.rs" <<'RS'
#[link(name = "m")]
#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

fn install_signal_handlers() {
    RECEIVED_SIGNAL.store(0, Ordering::Relaxed);
    unsafe {
        signal(2, record_signal);
        signal(15, record_signal);
    }
}
RS
    run bash "$SCRIPT" rank --findings "$WORK/raw.jsonl" --root "$WORK" --out "$WORK/attributed.jsonl"
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.dimension == "unsafe" and .file == "crates/autospec-cli/src/commands/runtime/env.rs")] | length' "$WORK/attributed.jsonl")" -eq 1 ]
}

@test "rank: reviewed unsafe approval is bound to exact path function and body" {
    local root="$WORK/reviewed-boundaries"
    local bridge="$root/crates/autospec-cli/src/commands/autonomous/executor_bridge.rs"
    run test -x "$SCRIPT"
    [ "$status" -eq 0 ]
    mkdir -p "$(dirname "$bridge")"
    : > "$WORK/empty.jsonl"
    cat > "$bridge" <<'EOF'
fn autonomous_executor_bridge_capture_and_reap_failure_retains_exact_quarantine() {
    let mut subreaper = 0_i32;
    let _ = unsafe {
            nix::libc::prctl(
                nix::libc::PR_GET_CHILD_SUBREAPER,
                std::ptr::addr_of_mut!(subreaper),
                0,
                0,
                0,
            )
    };
    let _duplicate = unsafe {
            nix::libc::prctl(
                nix::libc::PR_GET_CHILD_SUBREAPER,
                std::ptr::addr_of_mut!(subreaper),
                0,
                0,
                0,
            )
    };
}
fn wrong_location() {
    let mut value = 0_i32;
    let _ = unsafe {
            nix::libc::prctl(
                nix::libc::PR_GET_CHILD_SUBREAPER,
                std::ptr::addr_of_mut!(value),
                0,
                0,
                0,
            )
    };
}
fn autonomous_executor_bridge_clean_supervision_restores_prior_subreaper_state() {
    unsafe { altered_body(); }
    unsafe {
            nix::libc::prctl(
                nix::libc::PR_GET_CHILD_SUBREAPER,
                std::ptr::null_mut::<i32>(), 1, 2, 3)
    };
}
const MODULE_SCOPE_DUPLICATE: i32 = unsafe {
        nix::libc::prctl(
            nix::libc::PR_GET_CHILD_SUBREAPER,
            std::ptr::addr_of_mut!(subreaper),
            0,
            0,
            0,
        )
};
EOF
    run "$SCRIPT" rank --findings "$WORK/empty.jsonl" --root "$root" --out "$WORK/boundaries.jsonl"
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.dimension == "unsafe")] | length' "$WORK/boundaries.jsonl")" -eq 6 ]
}

@test "rank pr: compares unsafe syntax against renamed and edited baseline files" {
    run test -x "$SCRIPT"
    [ "$status" -eq 0 ]
    local repo="$WORK/repo"
    mkdir -p "$repo/src"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name "Security Workstream Test"
    : > "$WORK/empty.jsonl"
    cat > "$repo/src/lib.rs" <<'RS'
fn existing(ptr: *const i32) -> i32 { unsafe { std::ptr::read(ptr) } }
RS
    cp "$repo/src/lib.rs" "$repo/src/rename.rs"
    cp "$repo/src/lib.rs" "$repo/src/deleted.rs"
    printf '%s\n' 'fn formatting(ptr: *const i32) -> i32 { unsafe{ std::ptr::read(ptr) } }' > "$repo/src/format.rs"
    printf '%s\n' \
        'fn moved(ptr: *const i32) -> i32 { unsafe { std::ptr::read(ptr) } }' \
        'fn anchor() {}' > "$repo/src/internal-move.rs"
    printf '%s\n' 'fn x() {}' 'fn y() {}' 'fn z() {}' > "$repo/src/deletion-before-add.rs"
    printf '%s\n' 'fn replacement() -> i32 { 42 }' > "$repo/src/replacement.rs"
    cat > "$repo/src/attribute.rs" <<'RS'
#[
cfg(any())
]
pub extern "C" fn exported() {}
RS
    printf '%s\n' '{}' > "$repo/.security-workstream-added-lines.json"
    git -C "$repo" add .
    git -C "$repo" commit -qm "base"
    local base
    base="$(git -C "$repo" rev-parse HEAD)"

    perl -pi -e 's/fn existing/fn renamed/' "$repo/src/lib.rs"
    perl -pi -e 's/unsafe\{/unsafe {/' "$repo/src/format.rs"
    printf '%s\n' \
        'fn anchor() {}' \
        'fn moved(ptr: *const i32) -> i32 { unsafe { std::ptr::read(ptr) } }' > "$repo/src/internal-move.rs"
    printf '%s\n' 'unsafe fn z() {}' > "$repo/src/deletion-before-add.rs"
    perl -pi -e 'if ($. == 2) { s/cfg\(any\(\)\)/unsafe(no_mangle)/ }' "$repo/src/attribute.rs"
    printf '%s\n' 'fn replacement(ptr: *const i32) -> i32 { unsafe { std::ptr::read(ptr) } }' > "$repo/src/replacement.rs"
    git -C "$repo" mv src/rename.rs src/moved.rs
    git -C "$repo" rm -q src/deleted.rs
    git -C "$repo" add .
    git -C "$repo" commit -qm "modify unsafe contexts"

    run bash "$SCRIPT" rank --findings "$WORK/empty.jsonl" --root "$repo" --base "$base" --out "$WORK/pr.jsonl"
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.dimension == "unsafe")] | length' "$WORK/pr.jsonl")" -eq 3 ]
    jq -e 'select(.dimension == "unsafe" and .file == "src/attribute.rs" and .line == 2)' "$WORK/pr.jsonl" >/dev/null
    jq -e 'select(.dimension == "unsafe" and .file == "src/deletion-before-add.rs" and .line == 1)' "$WORK/pr.jsonl" >/dev/null
    jq -e 'select(.dimension == "unsafe" and .file == "src/replacement.rs" and .line == 1)' "$WORK/pr.jsonl" >/dev/null

    run bash "$SCRIPT" rank --findings "$WORK/empty.jsonl" --root "$repo" --out "$WORK/tree.jsonl"
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.dimension == "unsafe")] | length' "$WORK/tree.jsonl")" -eq 7 ]

    printf '%s\n' 'fn new(ptr: *const i32) -> i32 { unsafe { std::ptr::read(ptr) } }' > "$repo/src/untracked.rs"
    run bash "$SCRIPT" rank --findings "$WORK/empty.jsonl" --root "$repo" --base "$base" --out "$WORK/untracked.jsonl"
    [ "$status" -eq 0 ]
    [ "$(jq -s '[.[] | select(.dimension == "unsafe")] | length' "$WORK/untracked.jsonl")" -eq 4 ]

    run bash "$SCRIPT" rank --findings "$WORK/empty.jsonl" --root "$repo" --base missing-base --out "$WORK/invalid.jsonl"
    [ "$status" -eq 2 ]
    [[ "$output" == *"failed closed"* ]]
}

@test "issue filing: high-severity findings produce lint-clean remediation issues" {
    cat > "$WORK/ranked.jsonl" <<'JSONL'
{"gap_id":"G1","dimension":"secrets","severity":"must-fix","priority":"P0","severity_rank":100,"exploitability":5,"exposure":5,"file":"app/config.env","line":3,"title":"Hardcoded API token","body":"Remove the token and rotate the credential.","dedupe_key":"sec-secret","remediation":"Remove the committed token, rotate it, and add a regression scan."}
JSONL

    run bash "$SCRIPT" propose-issue --findings "$WORK/ranked.jsonl" --out "$WORK/issues"
    [ "$status" -eq 0 ]
    [ -f "$WORK/issues/p0-hardcoded-api-token.md" ]
    body="$(cat "$WORK/issues/p0-hardcoded-api-token.md")"
    [[ "$body" == *"auto-implement"* ]]
    [[ "$body" == *"priority:high"* ]]
    [[ "$body" == *"sec-secret"* ]]
    run bash "$REPO_ROOT/scripts/lint-issue.sh" "$WORK/issues/p0-hardcoded-api-token.md"
    [ "$status" -eq 0 ]
}

@test "headers: dashboard baseline gates missing CSP, HSTS, nosniff, referrer policy, and frame-ancestors" {
    cat > "$WORK/headers-ok.txt" <<'HEADERS'
Content-Security-Policy: default-src 'self'; frame-ancestors 'none'
Strict-Transport-Security: max-age=31536000; includeSubDomains
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
HEADERS
    run bash "$SCRIPT" check-headers --headers-file "$WORK/headers-ok.txt"
    [ "$status" -eq 0 ]
    [[ "$output" == *"security header baseline passed"* ]]

    cat > "$WORK/headers-bad.txt" <<'HEADERS'
Content-Security-Policy: default-src 'self'
X-Content-Type-Options: sniff
HEADERS
    run bash "$SCRIPT" check-headers --headers-file "$WORK/headers-bad.txt"
    [ "$status" -eq 1 ]
    [[ "$output" == *"missing Strict-Transport-Security"* ]]
    [[ "$output" == *"missing frame-ancestors"* ]]
    [[ "$output" == *"X-Content-Type-Options must be nosniff"* ]]
}

@test "verifier gate: security fixes cannot self-approve and sensitive domains require human review" {
    cat > "$WORK/fix-plan.json" <<'JSON'
{"author":"autospec-bot","verifier":"autospec-bot","touched_domains":["secrets"],"evidence":"bash scripts/security-workstream.sh check-headers --headers-file headers.txt"}
JSON
    run bash "$SCRIPT" verifier-gate --plan "$WORK/fix-plan.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"independent verifier required"* ]]
    [[ "$output" == *"human gate required: secrets"* ]]

    cat > "$WORK/fix-plan-ok.json" <<'JSON'
{"author":"autospec-bot","verifier":"security-reviewer","human_approved_by":"maintainer","touched_domains":["secrets"],"evidence":"bash scripts/security-workstream.sh rank --findings scan.jsonl --out ranked.jsonl"}
JSON
    run bash "$SCRIPT" verifier-gate --plan "$WORK/fix-plan-ok.json"
    [ "$status" -eq 0 ]
    [[ "$output" == *"security verifier gate passed"* ]]
}

@test "direct Rust validation gates the autonomous security workstream suite" {
    catalog="$REPO_ROOT/crates/autospec-core/src/validation/catalog.rs"
    grep -q '"check_autonomous_phase2_suite"' "$catalog"
    grep -q 'BatsDirectory("tests/autonomous")' "$catalog"
    [ -f "$REPO_ROOT/tests/autonomous/test_security_workstream.bats" ]
    grep -q 'schedule:' "$REPO_ROOT/.github/workflows/security-workstream.yml"
    grep -q 'pull_request:' "$REPO_ROOT/.github/workflows/security-workstream.yml"
}
