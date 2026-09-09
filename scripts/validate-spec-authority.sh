#!/usr/bin/env bash
# scripts/validate-spec-authority.sh — ratchet for spec-authority currency (#3947).
#
# A fleet that implemented one spec program while a newer charter for the same
# components sat unread in the tree merged forty times without anyone noticing.
# Nothing in the pipeline asked "which spec governs this component, and is that
# document still in force?". This script keeps the answer wired in:
#
#   1. the vocabulary and the gate exist where they are registered,
#   2. an unmarked document is never treated as current (verified by behavior,
#      not by reading the source),
#   3. a conflict between two spec programs is reported and stops the run
#      rather than being decided by an agent,
#   4. throughput is reported per authority, never as one aggregate number,
#   5. the CLI surface and the invariant are documented.
#
# Static checks always run. Behavior probes run when an `autospec` binary is
# available: `--bin <path>`, `$AUTOSPEC_BIN`, or `target/debug/autospec`
# built from this tree. With no binary the probes are skipped with a WARN and
# the static findings still decide the exit code.
#
# Usage:
#   scripts/validate-spec-authority.sh [--root DIR] [--bin PATH] [--quiet] [--help]
#
# Findings (stdout, one per line): SPEC_AUTHORITY:<code>: <detail>
#   REGISTRATION_MISSING   a component of the gate is absent or unwired
#   DOCUMENTATION_MISSING  a flag, the CLI row, or the invariant entry is undocumented
#   BEHAVIOR               a probe returned the wrong verdict or exit code
#   SKIP                   a probe could not run (informational, never counts)
#
# Exit 0 when clean; exit 1 on any finding (count capped at 64).

set -eu

script_dir="$(cd "${0%/*}" 2>/dev/null && pwd -P || pwd -P)"
root="$(cd "${script_dir}/.." && pwd -P)"
binary=""
quiet=0

usage() {
    awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --root) root="$2"; shift 2 ;;
        --bin) binary="$2"; shift 2 ;;
        --quiet) quiet=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) printf 'validate-spec-authority: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

findings=0
report() {
    printf 'SPEC_AUTHORITY:%s: %s\n' "$1" "$2"
    findings=$((findings + 1))
}
note() {
    [ "$quiet" -eq 1 ] || printf 'SPEC_AUTHORITY:SKIP: %s\n' "$1"
}

require_file() {
    [ -f "${root}/$1" ] || report REGISTRATION_MISSING "$1 is absent"
}

require_pattern() {
    # require_pattern <file> <pattern> <what-it-proves>
    if [ ! -f "${root}/$1" ]; then
        report REGISTRATION_MISSING "$1 is absent (needed for: $3)"
        return
    fi
    grep -qF -- "$2" "${root}/$1" || report REGISTRATION_MISSING "$1 lacks '$2' ($3)"
}

# ── 1. the gate is registered ────────────────────────────────────────────────
core=crates/autospec-core/src/spec_authority.rs
cli=crates/autospec-cli/src/commands/dispatch_authority.rs

require_file "$core"
require_file "$cli"
require_pattern crates/autospec-core/src/lib.rs "pub mod spec_authority;" "the core vocabulary is reachable"
require_pattern crates/autospec-cli/src/commands/mod.rs "pub mod dispatch_authority;" "the CLI module is compiled"
require_pattern crates/autospec-cli/src/commands/dispatch.rs '"authority"' "the subcommand is listed"
require_pattern crates/autospec-cli/src/commands/dispatch.rs "super::dispatch_authority::run(" "the subcommand is routed"
require_pattern crates/autospec-cli/src/commands/dispatch.rs \
    "Gate on the spec set in force before dispatching against it (#3947)" "the subcommand is listed in help"

for symbol in "pub enum CurrencyStatus" "pub enum AuthorityCode" "pub struct DispatchVerdict" \
              "pub fn parse_task_records" "pub fn throughput" "pub const UNDETERMINED"; do
    require_pattern "$core" "$symbol" "the gate vocabulary is public"
done

for code in CurrencyMissing SpecSuperseded AuthorityConflict VersionMissing; do
    require_pattern "$core" "$code" "every refusal reason has its own code"
done

# An unmarked document must resolve to Unknown, never to Current: the default
# is the whole bug. Pinned structurally here and behaviorally below.
require_pattern "$core" "unwrap_or(CurrencyStatus::Unknown)" "no currency marker resolves to Unknown"

# ── 2. the CLI surface is documented ─────────────────────────────────────────
cli_docs="${root}/docs/cli-reference.md"
require_pattern docs/cli-reference.md "autospec dispatch authority" "the command is in the CLI reference"
if [ -f "$cli_docs" ]; then
    for flag in --spec-dir --spec-file --component --tasks --json; do
        grep -A2 "autospec dispatch authority" "$cli_docs" | grep -qF -- "$flag" \
            || report DOCUMENTATION_MISSING "docs/cli-reference.md documents no '${flag}' for dispatch authority"
    done
fi
require_pattern docs/invariants.md "spec authority" "the invariant has an entry in docs/invariants.md"
require_pattern docs/invariants.md "#3947" "the invariant is traced to its issue"

# Cross-cutting invariants keep their rationale outside the file that implements
# them, and a ratchet that nothing invokes enforces nothing. Both are losable in
# silence, so they are pinned here.
require_pattern AGENTS.md "## Spec-authority dispatch gate" "the invariant's rationale lives in AGENTS.md"
require_pattern AGENTS.md "AUTHORITY_CONFLICT" "AGENTS.md states that a conflict is reported, never resolved"
require_pattern scripts/self-enforce-qa.sh "validate-spec-authority.sh" "the ratchet runs in the QA chain"

# ── 3. behavior probes ───────────────────────────────────────────────────────
if [ -z "$binary" ]; then
    if [ -n "${AUTOSPEC_BIN:-}" ]; then
        binary="$AUTOSPEC_BIN"
    elif [ -x "${root}/target/debug/autospec" ]; then
        binary="${root}/target/debug/autospec"
    else
        # A workspace built with CARGO_TARGET_DIR outside the checkout.
        binary="${CARGO_TARGET_DIR:-${root}/../target}/debug/autospec"
    fi
fi
if [ ! -x "$binary" ]; then
    note "no autospec binary at ${binary}; behavior probes skipped (build with cargo build -p autospec-cli)"
    if [ "$findings" -eq 0 ]; then exit 0; else c=$findings; [ "$c" -gt 64 ] && c=64; exit "$c"; fi
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

specs() { printf '%s' "$1" > "$work/spec.md"; }
run_gate() { "$binary" dispatch authority --spec-file "$work/spec.md" "$@" >"$work/out" 2>"$work/err"; }
output() { cat "$work/out" "$work/err"; }

probe() {
    # probe <name> <expected-exit> <expected-substring>
    local name="$1" want="$2" needle="$3" got
    run_gate && got=0 || got=$?
    if [ "$got" -ne "$want" ]; then
        report BEHAVIOR "$name exited $got, expected $want: $(output | head -1)"
        return
    fi
    output | grep -qF -- "$needle" || report BEHAVIOR "$name did not report '$needle'"
}

# A current set dispatches.
specs $'# Charter\nSpec-Set: program/a\nSpec-Version: V2\nAuthority-Over: gateway\n'
probe "current spec set" 0 "ALLOWED"
output | grep -qF "program/a" || report BEHAVIOR "an allowed dispatch does not name its authority"

# An unmarked document is never read as current.
specs $'# Charter\nThis charter describes the original architecture.\n'
probe "currency-less spec set" 1 "CURRENCY_MISSING"

# A superseded set refuses and names its successor.
specs $'# Charter\nSpec-Set: program/a\nSpec-Version: V1\nSuperseded-By: program/b\n'
probe "superseded spec set" 1 "SPEC_SUPERSEDED"
output | grep -qF "program/b" || report BEHAVIOR "a superseded verdict does not name the successor"

# Two programs claiming one component is reported, not decided.
printf '%s' $'# A\nSpec-Set: program/a\nSpec-Version: V2\nAuthority-Over: gateway\n' > "$work/a.md"
printf '%s' $'# B\nSpec-Set: program/b\nSpec-Version: V3\nAuthority-Over: gateway\n' > "$work/b.md"
"$binary" dispatch authority --spec-file "$work/a.md" --spec-file "$work/b.md" >"$work/out" 2>"$work/err" && got=0 || got=$?
[ "$got" -eq 1 ] || report BEHAVIOR "an authority conflict exited $got, expected 1"
output | grep -qF "AUTHORITY_CONFLICT" || report BEHAVIOR "a conflict was not reported as AUTHORITY_CONFLICT"
output | grep -qF "program/a" && output | grep -qF "program/b" \
    || report BEHAVIOR "a conflict does not name BOTH authorities"

# Task records report throughput per authority, and a record naming no
# authority is warned about instead of dropped.
printf '1\tprogram/a\tmerged\n2\tundetermined\tmerged\n' > "$work/tasks.tsv"
"$binary" dispatch authority --spec-file "$work/spec.md" --tasks "$work/tasks.tsv" >"$work/out" 2>"$work/err" && got=0 || got=$?
[ "$got" -eq 1 ] || report BEHAVIOR "a currency-less task authority set exited $got, expected 1"
grep -qF "THROUGHPUT BY SPEC AUTHORITY" "$work/out" \
    || report BEHAVIOR "no per-authority throughput section"
grep -qF "no determined spec authority" "$work/out" \
    || report BEHAVIOR "a task naming no authority is not warned about"

# A source that does not exist is unusable input (2), not a clean set (0).
"$binary" dispatch authority --spec-file "$work/absent.md" >/dev/null 2>&1 && got=0 || got=$?
[ "$got" -eq 2 ] || report BEHAVIOR "a missing source exited $got, expected 2"

if [ "$findings" -eq 0 ]; then
    [ "$quiet" -eq 1 ] || printf 'validate-spec-authority: OK (%s)\n' "registration, docs, behavior"
    exit 0
fi
c=$findings
[ "$c" -gt 64 ] && c=64
exit "$c"
