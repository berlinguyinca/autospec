#!/usr/bin/env bash
# autospec-worker-soak.sh — local fixture soak for concurrent autospec workers.

set -eu

usage() {
    cat <<'EOF'
Usage: autospec-worker-soak.sh --workers N --issues N [--keep]

Runs a local fixture-only concurrency soak against the Rust `autospec claim`
control plane. No GitHub network calls are made.
EOF
}

die() {
    printf 'autospec-worker-soak: %s\n' "$1" >&2
    exit 2
}

workers=""
issues=""
keep=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --workers) workers="${2:-}"; shift 2 ;;
        --issues) issues="${2:-}"; shift 2 ;;
        --keep) keep=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done

case "$workers" in *[!0-9]*|'') die "--workers must be a positive integer" ;; esac
case "$issues" in *[!0-9]*|'') die "--issues must be a positive integer" ;; esac
[ "$workers" -gt 0 ] || die "--workers must be a positive integer"
[ "$issues" -gt 0 ] || die "--issues must be a positive integer"

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
AUTOSPEC_CLAIM_BIN="${AUTOSPEC_BIN:-}"
if [ -z "$AUTOSPEC_CLAIM_BIN" ] && [ -x "$REPO_ROOT/target/debug/autospec" ]; then
    AUTOSPEC_CLAIM_BIN="$REPO_ROOT/target/debug/autospec"
fi
[ -n "$AUTOSPEC_CLAIM_BIN" ] || AUTOSPEC_CLAIM_BIN="autospec"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/autospec-worker-soak.XXXXXX")"
cleanup() {
    if [ "$keep" = "1" ]; then
        printf 'autospec-worker-soak: kept fixture dir: %s\n' "$tmp" >&2
    else
        rm -rf "$tmp" 2>/dev/null || true
    fi
}
trap cleanup EXIT

mkdir -p "$tmp/bin" "$tmp/issues"
calls="$tmp/calls.log"
claims="$tmp/claims.tsv"
: > "$calls"
: > "$claims"
printf '100\n' > "$tmp/next-id"

i=1
while [ "$i" -le "$issues" ]; do
    mkdir -p "$tmp/issues/$i"
    {
        printf 'auto-implement\n'
        printf 'ctx:32k\n'
        printf 'safety:reviewed\n'
    } > "$tmp/issues/$i/labels.txt"
    printf '[]\n' > "$tmp/issues/$i/comments.json"
    i=$((i + 1))
done

cat > "$tmp/bin/gh" <<'GH'
#!/usr/bin/env bash
set -eu

root="${AUTOSPEC_SOAK_ROOT:?}"
calls="${AUTOSPEC_SOAK_CALLS:?}"
printf '%s\n' "$*" >> "$calls"

issue_dir() {
    printf '%s/issues/%s' "$root" "$1"
}

with_lock() {
    lock="$1/.lock"
    while ! mkdir "$lock" 2>/dev/null; do
        sleep 0.01
    done
    trap 'rmdir "$lock" 2>/dev/null || true' EXIT
}

now_iso() {
    date -u +%Y-%m-%dT%H:%M:%SZ
}

if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
    printf 'testorg/testrepo\n'
    exit 0
fi

if [ "$1" = "label" ] && [ "$2" = "create" ]; then
    exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
    issue="$3"
    jq -Rn '
        [inputs | select(length > 0)] as $labels |
        {
          labels:$labels,
          body:"## Safety review\n\n<!-- autospec-safety:begin -->\n- **decision:** `SAFETY_PASS`\n<!-- autospec-safety:end -->\n",
          title:"Autospec soak issue",
          author:"autospec-soak"
        }
    ' < "$(issue_dir "$issue")/labels.txt"
    exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
    issue="$3"
    dir="$(issue_dir "$issue")"
    shift 3
    add=""
    remove=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --add-label) add="$2"; shift 2 ;;
            --remove-label) remove="$2"; shift 2 ;;
            *) shift ;;
        esac
    done
    with_lock "$dir"
    labels="$dir/labels.txt"
    if [ -n "$remove" ]; then
        tr ',' '\n' <<EOF | while IFS= read -r label; do
$remove
EOF
            [ -n "$label" ] || continue
            grep -Fxv "$label" "$labels" > "$labels.tmp" || true
            mv "$labels.tmp" "$labels"
        done
    fi
    if [ -n "$add" ]; then
        tr ',' '\n' <<EOF | while IFS= read -r label; do
$add
EOF
            [ -n "$label" ] || continue
            grep -Fx "$label" "$labels" >/dev/null 2>&1 || printf '%s\n' "$label" >> "$labels"
        done
    fi
    exit 0
fi

if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
    issue="$3"
    dir="$(issue_dir "$issue")"
    shift 3
    body=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --body) body="$2"; shift 2 ;;
            *) shift ;;
        esac
    done
    with_lock "$root"
    id="$(cat "$root/next-id")"
    printf '%s\n' "$((id + 1))" > "$root/next-id"
    jq --argjson id "$id" --arg body "$body" --arg updated_at "$(now_iso)" \
        '. + [{id:$id, updated_at:$updated_at, body:$body}]' \
        "$dir/comments.json" > "$dir/comments.tmp"
    mv "$dir/comments.tmp" "$dir/comments.json"
    exit 0
fi

if [ "$1" = "api" ]; then
    url="$2"
    shift 2
    method=""
    body=""
    while [ "$#" -gt 0 ]; do
        case "$1" in
            -X) method="$2"; shift 2 ;;
            -f) case "$2" in body=*) body="${2#body=}" ;; esac; shift 2 ;;
            --jq) shift 2 ;;
            *) shift ;;
        esac
    done
    case "$url" in
        repos/testorg/testrepo/issues/*/comments)
            issue="${url#repos/testorg/testrepo/issues/}"
            issue="${issue%/comments}"
            cat "$(issue_dir "$issue")/comments.json"
            ;;
        repos/testorg/testrepo/issues/comments/*)
            id="${url##*/}"
            for dir in "$root"/issues/*; do
                [ -d "$dir" ] || continue
                if jq -e --argjson id "$id" 'any(.[]; .id == $id)' "$dir/comments.json" >/dev/null; then
                    with_lock "$root"
                    case "$method" in
                        PATCH)
                            jq --argjson id "$id" --arg body "$body" --arg updated_at "$(now_iso)" \
                                'map(if .id == $id then .body = $body | .updated_at = $updated_at else . end)' \
                                "$dir/comments.json" > "$dir/comments.tmp"
                            mv "$dir/comments.tmp" "$dir/comments.json"
                            ;;
                        DELETE)
                            jq --argjson id "$id" 'map(select(.id != $id))' \
                                "$dir/comments.json" > "$dir/comments.tmp"
                            mv "$dir/comments.tmp" "$dir/comments.json"
                            ;;
                    esac
                    exit 0
                fi
            done
            ;;
    esac
    exit 0
fi

exit 1
GH
chmod +x "$tmp/bin/gh"

worker_loop() {
    worker="$1"
    issue=1
    while [ "$issue" -le "$issues" ]; do
        wid="soak-worker-${worker}"
        if AUTOSPEC_WORKER_ID="$wid" \
            PATH="$tmp/bin:$PATH" \
            AUTOSPEC_SOAK_ROOT="$tmp" \
            AUTOSPEC_SOAK_CALLS="$calls" \
            AUTOSPEC_HEARTBEAT_DIR="$tmp/heartbeats" \
            AUTOSPEC_CLAIM_RETRY_SLEEP_MS=0 \
            AUTOSPEC_CLAIM_CONFIRM_READS=1 \
            AUTOSPEC_CLAIM_SETTLE_MILLIS=0 \
            AUTOSPEC_CLAIM_LEASE_SECONDS=60 \
            "$AUTOSPEC_CLAIM_BIN" claim acquire --issue "$issue" --repo testorg/testrepo --worker-id "$wid" --branch "soak-$issue" >/dev/null 2>&1; then
            printf '%s\t%s\n' "$wid" "$issue" >> "$claims"
            AUTOSPEC_WORKER_ID="$wid" \
                PATH="$tmp/bin:$PATH" \
                AUTOSPEC_SOAK_ROOT="$tmp" \
                AUTOSPEC_SOAK_CALLS="$calls" \
                AUTOSPEC_HEARTBEAT_DIR="$tmp/heartbeats" \
                AUTOSPEC_CLAIM_RETRY_SLEEP_MS=0 \
                "$AUTOSPEC_CLAIM_BIN" claim release --issue "$issue" --repo testorg/testrepo --worker-id "$wid" --state merged --branch "soak-$issue" --pr "$issue" >/dev/null 2>&1 || true
        fi
        issue=$((issue + 1))
    done
}

w=1
while [ "$w" -le "$workers" ]; do
    worker_loop "$w" &
    w=$((w + 1))
done
wait

claim_count="$(wc -l < "$claims" | tr -d ' ')"
duplicate_claims="$(awk '{print $2}' "$claims" | sort | uniq -d | wc -l | tr -d ' ')"
stale_active=0
queue_remaining=0
i=1
while [ "$i" -le "$issues" ]; do
    labels="$tmp/issues/$i/labels.txt"
    if grep -Fx in-progress-by-bot "$labels" >/dev/null 2>&1; then
        stale_active=$((stale_active + 1))
    fi
    if grep -Fx auto-implement "$labels" >/dev/null 2>&1; then
        queue_remaining=$((queue_remaining + 1))
    fi
    i=$((i + 1))
done

status="pass"
if [ "$claim_count" -ne "$issues" ] || [ "$duplicate_claims" -ne 0 ] \
    || [ "$stale_active" -ne 0 ] || [ "$queue_remaining" -ne 0 ]; then
    status="fail"
fi

jq -n \
    --arg status "$status" \
    --argjson workers "$workers" \
    --argjson issues "$issues" \
    --argjson claims "$claim_count" \
    --argjson duplicate_claims "$duplicate_claims" \
    --argjson stale_active_labels "$stale_active" \
    --argjson queue_labels_remaining "$queue_remaining" \
    '{
      status: $status,
      workers: $workers,
      issues: $issues,
      claims: $claims,
      duplicate_claims: $duplicate_claims,
      stale_active_labels: $stale_active_labels,
      queue_labels_remaining: $queue_labels_remaining
    }'

[ "$status" = "pass" ]
