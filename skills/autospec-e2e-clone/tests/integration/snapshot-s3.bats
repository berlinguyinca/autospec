#!/usr/bin/env bats
# Integration tests for snapshot/s3.sh
# Requires docker + minio. Skip gracefully if unavailable.

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/snapshot"

MINIO_CONTAINER="autospec-test-minio-$$"
MINIO_PORT="19000"
MINIO_CONSOLE_PORT="19001"
MINIO_ROOT_USER="minioadmin"
MINIO_ROOT_PASSWORD="minioadmin"
MINIO_BUCKET="test-bucket"
export AWS_ENDPOINT_URL="http://127.0.0.1:${MINIO_PORT}"
export AWS_ACCESS_KEY_ID="$MINIO_ROOT_USER"
export AWS_SECRET_ACCESS_KEY="$MINIO_ROOT_PASSWORD"
export AWS_DEFAULT_REGION="us-east-1"
export BUCKET="$MINIO_BUCKET"

setup_file() {
    if ! command -v docker >/dev/null 2>&1; then
        return 0
    fi
    docker run -d \
        --name "$MINIO_CONTAINER" \
        -p "${MINIO_PORT}:9000" \
        -p "${MINIO_CONSOLE_PORT}:9001" \
        -e MINIO_ROOT_USER="$MINIO_ROOT_USER" \
        -e MINIO_ROOT_PASSWORD="$MINIO_ROOT_PASSWORD" \
        minio/minio:latest server /data --console-address ":9001" \
        >/dev/null 2>&1 || true

    # Wait for minio to be ready (up to 20s)
    local i=0
    while [ $i -lt 20 ]; do
        if curl -sf "http://127.0.0.1:${MINIO_PORT}/minio/health/live" >/dev/null 2>&1; then
            break
        fi
        sleep 1
        i=$((i + 1))
    done

    # Create bucket and seed test objects
    if command -v aws >/dev/null 2>&1; then
        aws --endpoint-url "$AWS_ENDPOINT_URL" s3 mb "s3://${MINIO_BUCKET}" >/dev/null 2>&1 || true
        for prefix in alpha beta gamma; do
            for i in $(seq 1 5); do
                printf 'object-%d\n' "$i" | \
                    aws --endpoint-url "$AWS_ENDPOINT_URL" s3 cp - \
                    "s3://${MINIO_BUCKET}/${prefix}/file_${i}.txt" >/dev/null 2>&1 || true
            done
        done
    fi
}

teardown_file() {
    if command -v docker >/dev/null 2>&1; then
        docker rm -f "$MINIO_CONTAINER" >/dev/null 2>&1 || true
    fi
}

setup() {
    TMP_DIR="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "s3.sh: creates out_dir and syncs objects" {
    require_minio
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/s3.sh" "100" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -d "$OUT_DIR" ]
    FILE_COUNT=$(find "$OUT_DIR" -type f | wc -l | tr -d ' ')
    [ "$FILE_COUNT" -gt 0 ]
}

@test "s3.sh: sampling cap limits objects per prefix" {
    require_minio
    OUT_DIR="$TMP_DIR/out"
    # Cap at 2 objects per prefix (each prefix has 5)
    run bash "$SCRIPT_DIR/s3.sh" "2" "$OUT_DIR"
    [ "$status" -eq 0 ]
    # Count files under alpha prefix — should be <= 2
    ALPHA_DIR="$OUT_DIR/alpha"
    if [ -d "$ALPHA_DIR" ]; then
        ALPHA_COUNT=$(find "$ALPHA_DIR" -type f | wc -l | tr -d ' ')
        [ "$ALPHA_COUNT" -le 2 ]
    fi
}

@test "s3.sh: fails with exit 2 if BUCKET not set" {
    OUT_DIR="$TMP_DIR/out"
    run env -u BUCKET bash "$SCRIPT_DIR/s3.sh" "100" "$OUT_DIR"
    [ "$status" -eq 2 ]
}

@test "s3.sh: fails with exit 1 if OUT_DIR not provided" {
    run bash "$SCRIPT_DIR/s3.sh"
    [ "$status" -eq 1 ]
}

@test "dispatch.sh: routes s3 source correctly" {
    require_minio
    OUT_BASE="$TMP_DIR/clone-snapshots"
    CONTRACT_JSON=$(cat <<JSON
{
  "sources": [
    {
      "kind": "s3",
      "bucket": "$MINIO_BUCKET",
      "sample_prefix_count": 3
    }
  ],
  "expose": { "kind": "docker_compose" }
}
JSON
)
    DISPATCH="$SCRIPT_DIR/dispatch.sh"
    run bash -c "printf '%s' '$CONTRACT_JSON' | bash '$DISPATCH' '$OUT_BASE'"
    [ "$status" -eq 0 ]
    FILE_COUNT=$(find "$OUT_BASE" -type f | wc -l | tr -d ' ')
    [ "$FILE_COUNT" -gt 0 ]
}

# Helpers
require_minio() {
    if ! command -v docker >/dev/null 2>&1; then
        skip "docker not available"
    fi
    if ! command -v aws >/dev/null 2>&1; then
        skip "aws CLI not available"
    fi
    if ! docker ps --filter "name=$MINIO_CONTAINER" --format "{{.Names}}" | grep -q "$MINIO_CONTAINER"; then
        skip "minio container not running"
    fi
    if ! curl -sf "http://127.0.0.1:${MINIO_PORT}/minio/health/live" >/dev/null 2>&1; then
        skip "minio not reachable"
    fi
}
