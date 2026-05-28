#!/usr/bin/env bash
# Validate autospec QA proof artifacts in a target repository.

set -eu

usage() {
    cat <<'EOF'
Usage: scripts/validate-qa-artifacts.sh [--repo DIR] [--schema-dir DIR]

Validates present QA artifacts under DIR/.autospec:
  proof-matrix.json
  reliability.yml or reliability.json
  control-intent-ledger.json
  mutation-proof.json
  canary-results.json

Requires ajv. YAML reliability contracts also require yq.
Missing artifacts are reported as WARN so draft-only/read-only QA reports can
still run this helper; malformed present artifacts fail.
EOF
}

repo="."
schema_dir="$(cd "$(dirname "$0")/.." && pwd)/schemas"

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo)
            repo="$2"
            shift 2
            ;;
        --schema-dir)
            schema_dir="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "validate-qa-artifacts: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

command -v ajv >/dev/null 2>&1 || {
    echo "validate-qa-artifacts: ajv is required" >&2
    exit 2
}

[ -d "$repo" ] || {
    echo "validate-qa-artifacts: repo directory not found: $repo" >&2
    exit 2
}

autospec_dir="$repo/.autospec"
tmpdir="$(mktemp -d /tmp/autospec-qa-artifacts-XXXXXX)"
trap 'rm -rf "$tmpdir"' EXIT

validate_json() {
    artifact="$1"
    schema="$2"
    if [ ! -f "$artifact" ]; then
        echo "WARN: missing $artifact" >&2
        return 0
    fi
    ajv validate -s "$schema" --spec=draft2020 -d "$artifact" >/dev/null
    echo "OK: $artifact"
}

validate_json \
    "$autospec_dir/proof-matrix.json" \
    "$schema_dir/autospec-proof-matrix.schema.json"
validate_json \
    "$autospec_dir/control-intent-ledger.json" \
    "$schema_dir/autospec-control-intent-ledger.schema.json"
validate_json \
    "$autospec_dir/mutation-proof.json" \
    "$schema_dir/autospec-mutation-proof.schema.json"
validate_json \
    "$autospec_dir/canary-results.json" \
    "$schema_dir/autospec-canary-results.schema.json"

if [ -f "$autospec_dir/reliability.json" ]; then
    validate_json \
        "$autospec_dir/reliability.json" \
        "$schema_dir/autospec-reliability.schema.json"
elif [ -f "$autospec_dir/reliability.yml" ]; then
    command -v yq >/dev/null 2>&1 || {
        echo "validate-qa-artifacts: yq is required for $autospec_dir/reliability.yml" >&2
        exit 2
    }
    yq -o=json '.' "$autospec_dir/reliability.yml" > "$tmpdir/reliability.json"
    ajv validate \
        -s "$schema_dir/autospec-reliability.schema.json" \
        --spec=draft2020 \
        -d "$tmpdir/reliability.json" >/dev/null
    echo "OK: $autospec_dir/reliability.yml"
else
    echo "WARN: missing $autospec_dir/reliability.yml" >&2
fi
