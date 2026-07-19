#!/usr/bin/env bats
# tests/autospec/test_quality_audit_summary.bats

setup() {
    HELPER="${BATS_TEST_DIRNAME}/../../scripts/autospec-write-run-summary.sh"
    TMP_DIR="$(mktemp -d)"
    OUT="$TMP_DIR/run-summary.md"
    AUDIT="$TMP_DIR/quality-audit.json"
    cat > "$AUDIT" <<'EOF'
{
  "status": "fail",
  "summary": {
    "total_findings": 3,
    "suppressed_findings": 1,
    "issue_links": 1,
    "unfiled_residual_risks": 2
  },
  "runtime": {
    "node": {"version": "20.11.1", "engine": ">=20", "status": "passed"},
    "package_managers": {
      "npm": {"version": "10.2.4", "engine": ">=10", "status": "passed"},
      "pnpm": {"version": null, "engine": null, "status": "not configured"},
      "yarn": {"version": null, "engine": null, "status": "not configured"}
    }
  },
  "artifacts": {
    "npm_audit": ".autospec/artifacts/npm-audit.json"
  },
  "verification": {
    "lanes": {
      "lint": {"status": "configured but failing", "command": "npm run lint"},
      "test": {"status": "passed", "command": "npm run test"},
      "typecheck": {"status": "not run", "command": "npm run typecheck"},
      "audit": {"status": "not configured", "command": null}
    }
  },
  "issue_links": [
    {"title": "autospec audit: missing lint script", "url": "https://github.com/example/repo/issues/99"}
  ],
  "suppressed": [
    {"title": "legacy debug logging", "dedupe_key": "debug-logging-hotspots:src/legacy.js"}
  ],
  "residual_risks": [
    "Unfiled app-follow-up: route coverage gaps",
    "Unfiled autospec process gap: missing dependency audit script"
  ]
}
EOF
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "run summary includes repo quality audit status, issue links, suppressed findings, and residual risks" {
    CHALLENGE="$TMP_DIR/challenge.md"
    printf -- '- Done verdict: reviewed.\n' > "$CHALLENGE"
    run bash "$HELPER" \
        --done-challenge-file "$CHALLENGE" \
        --quality-audit-json "$AUDIT" \
        --output "$OUT" \
        --sha abc123 \
        --branch main
    [ "$status" -eq 0 ]
    grep -q '^## Repo quality audit' "$OUT"
    grep -qF -- '- Status: fail' "$OUT"
    grep -qF -- '- Findings: 3' "$OUT"
    grep -qF -- '- Filed issues: 1' "$OUT"
    grep -qF -- '### Runtime and engines' "$OUT"
    grep -qF -- '- node: 20.11.1 (engine: >=20, status: passed)' "$OUT"
    grep -qF -- '- npm: 10.2.4 (engine: >=10, status: passed)' "$OUT"
    grep -qF -- '### Artifacts' "$OUT"
    grep -qF -- '- npm_audit: `.autospec/artifacts/npm-audit.json`' "$OUT"
    grep -qF -- '### Verification lanes' "$OUT"
    grep -qF -- '- lint: configured but failing (`npm run lint`)' "$OUT"
    grep -qF -- '- test: passed (`npm run test`)' "$OUT"
    grep -qF -- '- typecheck: not run (`npm run typecheck`)' "$OUT"
    grep -qF -- '- audit: not configured' "$OUT"
    grep -qF -- '- autospec audit: missing lint script — https://github.com/example/repo/issues/99' "$OUT"
    grep -qF -- '- legacy debug logging (`debug-logging-hotspots:src/legacy.js`)' "$OUT"
    grep -qF -- '- Unfiled app-follow-up: route coverage gaps' "$OUT"
}
