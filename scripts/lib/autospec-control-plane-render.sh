#!/usr/bin/env bash
# Render helpers for scripts/autospec-control-plane.sh.

print_group() {
    group="$1"
    shift
    printf '  %s/\n' "$group"
    for item in "$@"; do
        printf '    %s/%s\n' "$group" "$item"
    done
}

render_file_header() {
    repo="$1"
    path="$2"
    printf '\n--- %s/%s ---\n' "$repo" "$path"
}

render_policy_schema() {
    cat <<'JSON'
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://autospec.dev/schemas/policy.schema.json",
  "title": "Autospec governance policy pack",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "policy_id",
    "version",
    "project_class",
    "privacy_tier",
    "priority_waterfall",
    "merge_rules",
    "cost_limits",
    "evidence_requirements"
  ],
  "properties": {
    "policy_id": {"type": "string", "pattern": "^[a-z0-9][a-z0-9-]*$"},
    "version": {"type": "integer", "minimum": 1},
    "project_class": {"enum": ["open-source", "private-personal", "private-company", "client-project", "research", "sandbox"]},
    "privacy_tier": {"enum": ["metadata-only", "summary", "evidence", "full-debug"]},
    "raw_logs_allowed": {"type": "boolean"},
    "priority_waterfall": {"type": "array", "items": {"type": "string"}, "minItems": 1},
    "merge_rules": {"type": "object"},
    "cost_limits": {"type": "object"},
    "evidence_requirements": {"type": "object"},
    "rules": {"type": "array", "items": {"type": "string"}}
  }
}
JSON
}

render_rule_schema() {
    cat <<'JSON'
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://autospec.dev/schemas/rule.schema.json",
  "title": "Autospec governance rule catalog",
  "type": "object",
  "additionalProperties": false,
  "required": ["catalog_id", "version", "rules"],
  "properties": {
    "catalog_id": {"type": "string"},
    "version": {"type": "integer", "minimum": 1},
    "rules": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["rule_id", "category", "severity", "deterministic_checks"],
        "properties": {
          "rule_id": {"type": "string"},
          "category": {"enum": ["priority", "privacy", "merge", "cost", "evidence", "quality"]},
          "severity": {"enum": ["info", "warn", "block"]},
          "deterministic_checks": {"type": "array", "items": {"type": "string"}, "minItems": 1}
        }
      }
    }
  }
}
JSON
}

render_project_class_schema() {
    cat <<'JSON'
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://autospec.dev/schemas/project-class.schema.json",
  "type": "object",
  "required": ["project_id", "project_class", "default_policy"],
  "properties": {
    "project_id": {"type": "string"},
    "project_class": {"enum": ["open-source", "private-personal", "private-company", "client-project", "research", "sandbox"]},
    "default_policy": {"type": "string"}
  }
}
JSON
}

render_priority_schema() {
    cat <<'JSON'
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://autospec.dev/schemas/priority.schema.json",
  "type": "object",
  "required": ["priority_waterfall"],
  "properties": {
    "priority_waterfall": {"type": "array", "items": {"type": "string"}, "minItems": 1}
  }
}
JSON
}

render_policy_pack() {
    policy_id="$1"
    project_class="$2"
    privacy_tier="$3"
    raw_logs_allowed="$4"
    daily_usd="$5"
    priority_csv="$6"

    cat <<EOF_POLICY
policy_id: $policy_id
version: 1
project_class: $project_class
privacy_tier: $privacy_tier
raw_logs_allowed: $raw_logs_allowed
priority_waterfall:
EOF_POLICY
    old_ifs="$IFS"
    IFS=','
    for priority in $priority_csv; do
        printf '  - %s\n' "$priority"
    done
    IFS="$old_ifs"
    cat <<EOF_POLICY
merge_rules:
  require_ci_green: true
  require_review_lgtm: true
  allow_admin_merge: false
cost_limits:
  daily_usd: $daily_usd
  stop_on_budget_exhaustion: true
evidence_requirements:
  runtime_proof_required: true
  closeout_report_required: true
  policy_trace_required: true
rules:
  - rules/qa.yml
  - rules/testing.yml
  - rules/documentation.yml
  - rules/security.yml
  - rules/accessibility.yml
  - rules/performance.yml
  - rules/skill-generation.yml
  - rules/release-readiness.yml
EOF_POLICY
}

render_rule_catalog() {
    catalog_id="$1"
    rule_id="$2"
    category="$3"
    check="$4"

    cat <<EOF_RULE
catalog_id: $catalog_id
version: 1
rules:
  - rule_id: $rule_id
    category: $category
    severity: block
    deterministic_checks:
      - $check
EOF_RULE
}

render_project_fixture() {
    project_id="$1"
    project_class="$2"
    default_policy="$3"

    cat <<EOF_FIXTURE
project_id: $project_id
project_class: $project_class
default_policy: $default_policy
repository_visibility: fixture
policy_resolution:
  digest_required: true
  trace_required: true
EOF_FIXTURE
}

render_schema_templates() {
    governance_repo="$1"

    render_file_header "$governance_repo" "schemas/policy.schema.json"
    render_policy_schema
    render_file_header "$governance_repo" "schemas/rule.schema.json"
    render_rule_schema
    render_file_header "$governance_repo" "schemas/project-class.schema.json"
    render_project_class_schema
    render_file_header "$governance_repo" "schemas/priority.schema.json"
    render_priority_schema
}

render_policy_templates() {
    governance_repo="$1"

    render_file_header "$governance_repo" "policies/open-source-maintainer-default.yml"
    render_policy_pack "open-source-maintainer-default" "open-source" "summary" "false" "25" "security,ci-health,tests,docs,release-hygiene,accessibility"
    render_file_header "$governance_repo" "policies/private-personal-default.yml"
    render_policy_pack "private-personal-default" "private-personal" "evidence" "false" "15" "velocity,learning,exploration,tests,docs"
    render_file_header "$governance_repo" "policies/private-company-default.yml"
    render_policy_pack "private-company-default" "private-company" "summary" "false" "100" "qa,business-workflows,deployment-health,internal-docs,security,cost"
    render_file_header "$governance_repo" "policies/client-project-default.yml"
    render_policy_pack "client-project-default" "client-project" "metadata-only" "false" "75" "audit-trail,scope-boundaries,cost-attribution,report-exports,privacy"
    render_file_header "$governance_repo" "policies/research-default.yml"
    render_policy_pack "research-default" "research" "summary" "false" "50" "reproducibility,notebooks,data-safety,provenance,result-docs"
    render_file_header "$governance_repo" "policies/sandbox-default.yml"
    render_policy_pack "sandbox-default" "sandbox" "evidence" "true" "10" "experimentation,debug-capture,cleanup,cost-limits"
}

render_rule_templates() {
    governance_repo="$1"

    render_file_header "$governance_repo" "rules/qa.yml"
    render_rule_catalog "qa" "qa-runtime-proof" "evidence" "runtime proof exists for runtime claims"
    render_file_header "$governance_repo" "rules/testing.yml"
    render_rule_catalog "testing" "testing-priority-waterfall" "priority" "tests precede implementation for behavior changes"
    render_file_header "$governance_repo" "rules/documentation.yml"
    render_rule_catalog "documentation" "documentation-public-surface" "evidence" "public surface changes cite matching docs"
    render_file_header "$governance_repo" "rules/security.yml"
    render_rule_catalog "security" "security-privacy-tier" "privacy" "raw logs are blocked unless policy allows them"
    render_file_header "$governance_repo" "rules/accessibility.yml"
    render_rule_catalog "accessibility" "accessibility-evidence" "evidence" "UI changes include accessibility evidence"
    render_file_header "$governance_repo" "rules/performance.yml"
    render_rule_catalog "performance" "performance-cost-limit" "cost" "runs stop when policy cost ceiling is exhausted"
    render_file_header "$governance_repo" "rules/skill-generation.yml"
    render_rule_catalog "skill-generation" "skill-generation-merge-gate" "merge" "generated skills pass lock-step validation before merge"
    render_file_header "$governance_repo" "rules/release-readiness.yml"
    render_rule_catalog "release-readiness" "release-readiness-merge-gate" "merge" "release PRs require green validation before merge"
}

render_project_fixtures() {
    governance_repo="$1"

    render_file_header "$governance_repo" "fixtures/projects/open-source-cli.yml"
    render_project_fixture "open-source-cli" "open-source" "open-source-maintainer-default"
    render_file_header "$governance_repo" "fixtures/projects/private-personal-app.yml"
    render_project_fixture "private-personal-app" "private-personal" "private-personal-default"
    render_file_header "$governance_repo" "fixtures/projects/private-company-saas.yml"
    render_project_fixture "private-company-saas" "private-company" "private-company-default"
    render_file_header "$governance_repo" "fixtures/projects/client-webapp.yml"
    render_project_fixture "client-webapp" "client-project" "client-project-default"
    render_file_header "$governance_repo" "fixtures/projects/research-notebook.yml"
    render_project_fixture "research-notebook" "research" "research-default"
    render_file_header "$governance_repo" "fixtures/projects/sandbox-lab.yml"
    render_project_fixture "sandbox-lab" "sandbox" "sandbox-default"
}

render_governance_file_templates() {
    governance_repo="$1"

    render_schema_templates "$governance_repo"
    render_policy_templates "$governance_repo"
    render_rule_templates "$governance_repo"
    render_project_fixtures "$governance_repo"
}
