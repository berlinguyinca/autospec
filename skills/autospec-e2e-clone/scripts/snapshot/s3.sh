#!/usr/bin/env bash
# skills/autospec-e2e-clone/scripts/snapshot/s3.sh — S3 snapshot driver
#
# Usage: s3.sh <sample_prefix_count> <out_dir>
#
# Environment:
#   BUCKET              — S3 bucket name (required)
#   AWS_PROFILE         — optional AWS profile
#   AWS_ENDPOINT_URL    — optional endpoint override (for minio/localstack)
#
# Behavior:
#   Lists top-level prefixes in BUCKET, then for each prefix syncs up to
#   sample_prefix_count objects into <out_dir>/<prefix>/
#
# Output layout:
#   <out_dir>/<prefix>/...  — synced objects per prefix
#
# Exit codes:
#   0 = ok
#   1 = fatal (missing tool, aws error)
#   2 = refuse-to-run (operator-actionable config error)

set -euo pipefail

SAMPLE_PREFIX_COUNT="${1:-100}"
OUT_DIR="${2:-}"

# ── Validate arguments ────────────────────────────────────────────────────────
if [ -z "$OUT_DIR" ]; then
    printf 's3.sh: fatal: OUT_DIR argument required\n' >&2
    exit 1
fi

if [ -z "${BUCKET:-}" ]; then
    printf 's3.sh: refuse-to-run: BUCKET environment variable not set\n' >&2
    printf 'Set BUCKET to the S3 bucket name to snapshot\n' >&2
    exit 2
fi

# ── Dependency check ──────────────────────────────────────────────────────────
if ! command -v aws >/dev/null 2>&1; then
    printf 's3.sh: fatal: aws CLI not found. Install awscli\n' >&2
    exit 1
fi

# ── Build aws base args ───────────────────────────────────────────────────────
AWS_ARGS=()
if [ -n "${AWS_ENDPOINT_URL:-}" ]; then
    AWS_ARGS+=(--endpoint-url "$AWS_ENDPOINT_URL")
fi

# ── Prepare output directory ──────────────────────────────────────────────────
mkdir -p "$OUT_DIR"

# ── List top-level prefixes ───────────────────────────────────────────────────
printf 's3.sh: listing prefixes in s3://%s...\n' "$BUCKET" >&2

PREFIXES=$(aws "${AWS_ARGS[@]}" s3api list-objects-v2 \
    --bucket "$BUCKET" \
    --delimiter "/" \
    --query "CommonPrefixes[].Prefix" \
    --output text 2>&1) || {
    printf 's3.sh: fatal: failed to list prefixes in bucket %s\n' "$BUCKET" >&2
    exit 1
}

if [ -z "$PREFIXES" ] || [ "$PREFIXES" = "None" ]; then
    # No prefixes — treat entire bucket as flat, sync with object cap
    printf 's3.sh: no prefixes found; syncing flat bucket (cap: %s objects)...\n' "$SAMPLE_PREFIX_COUNT" >&2
    KEYS=$(aws "${AWS_ARGS[@]}" s3api list-objects-v2 \
        --bucket "$BUCKET" \
        --max-items "$SAMPLE_PREFIX_COUNT" \
        --query "Contents[].Key" \
        --output text 2>/dev/null || echo "")
    if [ -n "$KEYS" ] && [ "$KEYS" != "None" ]; then
        while IFS=$'\t' read -r key; do
            [ -z "$key" ] && continue
            aws "${AWS_ARGS[@]}" s3 cp "s3://${BUCKET}/${key}" "$OUT_DIR/${key}" \
                --quiet 2>&1 || printf 's3.sh: warning: failed to copy %s\n' "$key" >&2
        done <<< "$KEYS"
    fi
    printf 's3.sh: snapshot complete → %s\n' "$OUT_DIR" >&2
    exit 0
fi

# ── Sync per prefix with cap ──────────────────────────────────────────────────
PREFIX_COUNT=0
while IFS=$'\t' read -r prefix; do
    [ -z "$prefix" ] && continue
    PREFIX_DIR="${OUT_DIR}/${prefix%/}"
    mkdir -p "$PREFIX_DIR"

    printf 's3.sh: syncing prefix %s (cap: %s)...\n' "$prefix" "$SAMPLE_PREFIX_COUNT" >&2

    # List objects under this prefix, cap to sample_prefix_count
    KEYS=$(aws "${AWS_ARGS[@]}" s3api list-objects-v2 \
        --bucket "$BUCKET" \
        --prefix "$prefix" \
        --max-items "$SAMPLE_PREFIX_COUNT" \
        --query "Contents[].Key" \
        --output text 2>/dev/null || echo "")

    if [ -n "$KEYS" ] && [ "$KEYS" != "None" ]; then
        while IFS=$'\t' read -r key; do
            [ -z "$key" ] && continue
            rel_key="${key#$prefix}"
            aws "${AWS_ARGS[@]}" s3 cp "s3://${BUCKET}/${key}" "${PREFIX_DIR}/${rel_key}" \
                --quiet 2>&1 || printf 's3.sh: warning: failed to copy %s\n' "$key" >&2
        done <<< "$KEYS"
    fi

    PREFIX_COUNT=$((PREFIX_COUNT + 1))
done <<< "$PREFIXES"

printf 's3.sh: snapshot complete — %d prefix(es) → %s\n' "$PREFIX_COUNT" "$OUT_DIR" >&2
