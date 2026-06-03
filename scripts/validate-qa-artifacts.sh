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
# Schema directory resolution (issue #856):
#   1. AUTOSPEC_SCHEMAS_DIR env override (set by operators or tests).
#   2. --schema-dir CLI flag (explicit override, takes precedence over env).
#   3. Default: ~/.autospec/schemas/ (the installed location populated by install.sh).
# When running from a repo checkout, scripts/validate-qa-artifacts.sh resolves
# to repo/schemas/ via the relative "$(dirname $0)/.." path; from the installed
# copy (~/.autospec/scripts/), the env/default resolves correctly without a
# checkout alongside the scripts.
schema_dir="${AUTOSPEC_SCHEMAS_DIR:-$HOME/.autospec/schemas}"

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

# Guard: verify the schema directory exists and contains the required schemas.
# Print an actionable error and exit non-zero instead of silently skipping QA
# validation, so operators know exactly how to recover (run install.sh --update).
_required_schemas="
autospec-proof-matrix.schema.json
autospec-control-intent-ledger.schema.json
autospec-mutation-proof.schema.json
autospec-canary-results.schema.json
autospec-reliability.schema.json
"
_missing_schemas=""
for _schema in $_required_schemas; do
    [ -z "$_schema" ] && continue
    if [ ! -f "$schema_dir/$_schema" ]; then
        _missing_schemas="$_missing_schemas $_schema"
    fi
done
if [ -n "$_missing_schemas" ]; then
    echo "validate-qa-artifacts: schemas directory is missing required files: $_missing_schemas" >&2
    echo "  Looked in: $schema_dir" >&2
    echo "  Recovery:  run 'bash install.sh --update' to install schemas to \$AUTOSPEC_SCHEMAS_DIR (default ~/.autospec/schemas/)" >&2
    echo "  Override:  set AUTOSPEC_SCHEMAS_DIR or pass --schema-dir to point at the correct location" >&2
    exit 3
fi
unset _required_schemas _missing_schemas _schema

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

yaml_to_json() {
    src="$1"
    dest="$2"
    if yq -o=json '.' "$src" > "$dest" 2>/dev/null; then
        return 0
    fi
    # Python yq transcodes YAML to JSON by default and forwards -o to jq.
    yq '.' "$src" > "$dest"
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
    yaml_to_json "$autospec_dir/reliability.yml" "$tmpdir/reliability.json"
    ajv validate \
        -s "$schema_dir/autospec-reliability.schema.json" \
        --spec=draft2020 \
        -d "$tmpdir/reliability.json" >/dev/null
    echo "OK: $autospec_dir/reliability.yml"
else
    echo "WARN: missing $autospec_dir/reliability.yml" >&2
fi
