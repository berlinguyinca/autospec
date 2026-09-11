#!/usr/bin/env bats
# Runbook contract for the refresh-queue sweep (issue #3978).
#
# A negative finding lost its scope qualifier and became a false premise:
# "refresh-queue.sh does not exist", when the tool lives in the repository
# checkout on the local host and simply does not exist on the cluster.
# These tests pin the contract that makes that failure impossible:
#
#   - the runbook documents an entry point that resolves to a real file,
#     taken from the runbook alone (the populated case: the check runs
#     against the real repository, the same way
#     tests/autospec-log-status.bats covers the populated #3793 case);
#   - the sweep is scheduled, alongside topup, on a shared cadence;
#   - the runbook records the location and the off-cluster reason;
#   - the operator-loop index names every tool with host, cadence, and a
#     resolvable repo-relative entry point.

RUNBOOK="docs/runbooks/refresh-queue-sweep.md"
INDEX="docs/runbooks/operator-loop-tools.md"

# entry_point <runbook>
# Print the entry point the runbook documents. Fails when the runbook names
# no entry point — a runbook that cannot be followed is the #3978 failure.
entry_point() {
    local runbook="$1"
    local ep
    ep="$(grep -oE '\*\*Entry point:\*\* `[^`]+`' "$runbook" | head -n1 | sed -E 's/.*`([^`]+)`$/\1/')"
    [ -n "$ep" ] && printf '%s\n' "$ep"
}

# cron_cadence <runbook> <step>
# Print the minute+hour fields of the crontab line that schedules <step>.
cron_cadence() {
    local runbook="$1" step="$2"
    awk '/^```cron$/,/^```$/' "$runbook" \
        | grep -F "$step" | head -n1 | awk '{print $1, $2}'
}

@test "runbook documents an entry point that resolves to a real file (populated case)" {
    [ -f "$RUNBOOK" ]
    local ep
    ep="$(entry_point "$RUNBOOK")"
    [ -n "$ep" ]
    # The path, taken from the runbook alone, is repo-relative and resolves
    # to a real file in the checkout.
    case "$ep" in
        /*|~*) echo "entry point $ep is not repo-relative" >&2; return 1 ;;
    esac
    [ -f "$ep" ]
    # The runbook's tests pointer resolves too.
    grep -q 'tests/refresh-queue.bats' "$RUNBOOK"
    [ -f tests/refresh-queue.bats ]
}

@test "a runbook whose entry point does not resolve is a failure" {
    local tmp ep
    tmp="$(mktemp)"
    printf '%s\n' '**Entry point:** `scripts/does-not-exist-3978.sh`' > "$tmp"
    ep="$(entry_point "$tmp")"
    [ "$ep" = "scripts/does-not-exist-3978.sh" ]
    run test -f "$ep"
    [ "$status" -ne 0 ]
    rm -f "$tmp"
}

@test "a runbook that names no entry point is a failure" {
    local tmp
    tmp="$(mktemp)"
    printf '%s\n' '# A runbook with no documented entry point' > "$tmp"
    run entry_point "$tmp"
    [ "$status" -ne 0 ]
    rm -f "$tmp"
}

@test "crontab schedules refresh-queue on the same cadence as topup" {
    [ -f "$RUNBOOK" ]
    local refresh topup
    refresh="$(cron_cadence "$RUNBOOK" refresh-queue)"
    topup="$(cron_cadence "$RUNBOOK" topup)"
    [ -n "$refresh" ]
    [ "$refresh" = "$topup" ]
    # */10 = the topology default (DEFAULT_INTERVAL_SECS 600s).
    [ "$refresh" = "*/10 *" ]
}

@test "runbook records the location and the off-cluster reason" {
    [ -f "$RUNBOOK" ]
    grep -q '^## Where the tool lives' "$RUNBOOK"
    grep -q '^## Why it must run off-cluster' "$RUNBOOK"
    # The reason, not just the location: gh is unauthenticated on the
    # cluster, so a credential-holding step cannot run there.
    grep -qi 'unauthenticated on the cluster' "$RUNBOOK"
    grep -qi 'credential' "$RUNBOOK"
}

@test "operator-loop index names every tool with host, cadence, and resolvable entry point" {
    [ -f "$INDEX" ]
    # Table columns: | Tool | Entry point | Host | Cadence | Role | Runbook |
    # Fields under -F'|': $2 tool, $3 entry point, $5 cadence.
    local rows
    rows="$(awk -F'|' '
        {
            name = $2; ep = $3; cad = $5
            gsub(/^[ \t]+|[ \t]+$/, "", name)
            gsub(/^[ \t]+|[ \t]+$/, "", ep)
            gsub(/^[ \t]+|[ \t]+$/, "", cad)
            if (name == "" || name ~ /^-+$/) next
            print name "\t" ep "\t" cad
        }' "$INDEX")"
    [ -n "$rows" ]
    # The two hops that must share a cadence are both present, and they do.
    local refresh_cad topup_cad
    refresh_cad="$(printf '%s\n' "$rows" | awk -F'\t' '$1 == "refresh-queue" { print $3 }')"
    topup_cad="$(printf '%s\n' "$rows" | awk -F'\t' '$1 == "topup" { print $3 }')"
    [ -n "$refresh_cad" ]
    [ -n "$topup_cad" ]
    [ "$refresh_cad" = "$topup_cad" ]
    # Every repo-relative entry point resolves to a real file. Deployment
    # paths (<...>), home paths, absolute paths, and prose are skipped.
    local name ep
    while IFS=$'\t' read -r name ep _; do
        ep="${ep//\`/}"
        case "$ep" in
            *"<"* | "~"* | /* | *" "* | "") continue ;;
        esac
        if [ ! -f "$ep" ]; then
            echo "index entry point for $name does not resolve: $ep" >&2
            return 1
        fi
    done <<< "$rows"
}
