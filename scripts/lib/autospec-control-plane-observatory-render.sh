#!/usr/bin/env bash
# Observatory render helpers for scripts/autospec-control-plane.sh.

CONTROL_PLANE_OBSERVATORY_EVENTS_RENDER_LIB="${CONTROL_PLANE_OBSERVATORY_EVENTS_RENDER_LIB:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/autospec-control-plane-observatory-events-render.sh}"
# shellcheck source=scripts/lib/autospec-control-plane-observatory-events-render.sh
. "$CONTROL_PLANE_OBSERVATORY_EVENTS_RENDER_LIB"
CONTROL_PLANE_OBSERVATORY_UI_RENDER_LIB="${CONTROL_PLANE_OBSERVATORY_UI_RENDER_LIB:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/autospec-control-plane-observatory-ui-render.sh}"
# shellcheck source=scripts/lib/autospec-control-plane-observatory-ui-render.sh
. "$CONTROL_PLANE_OBSERVATORY_UI_RENDER_LIB"

render_observatory_compose() {
    cat <<'YAML'
services:
  postgres:
    image: postgres:16
    container_name: observatory-postgres
    ports:
      - "5432:5432"
    volumes:
      - observatory-postgres-data:/var/lib/postgresql/data

volumes:
  observatory-postgres-data:
YAML
}

render_observatory_api_readme() {
    cat <<'MD'
# Autospec Observatory API

Seed path for the future event ingestion API. The MVP contract is
API-key authenticated ingestion for `POST /v1/events/batch`, project resolution,
run progress via `GET /v1/runs/:id/progress`, timeline, worker, policy, cost,
and fleet summary endpoints.

This dry-run scaffold is offline-only and does not implement or start a live
server.
MD
}

render_observatory_web_readme() {
    cat <<'MD'
# Autospec Observatory Web

Seed path for the developer/operator UI. The MVP UI will expose fleet, timeline,
work-item, blockers, workers, policy-decision, cost, duration, outcome, and
progress views with 10-second polling by default.

This dry-run scaffold is offline-only and does not implement or start a web app.
MD
}

render_observatory_event_schema_readme() {
    cat <<'MD'
# Event Schema Package

Seed package for run, cycle, work-item, worker, cost, blocker, progress, and
policy-decision events. Events must carry project classification and
`privacy_tier` metadata so client and server privacy enforcement can reject
over-shared payloads.

Duplicate event_id is ignored. `sequence` is monotonic per run_id; repeated
sequences are stored once and flagged, sequence gaps are exposed for UI review,
and late events are stored by occurred_at and received_at.
MD
}

render_observatory_db_readme() {
    cat <<'MD'
# Database Package

Seed package for the future Postgres-backed multi-tenant data model covering
runs, events, workers, projects, repositories, costs, blockers, and policy
decisions.
MD
}

render_observatory_ui_readme() {
    cat <<'MD'
# UI Package

Seed package for shared web UI components used by the observatory operator
console.
MD
}

render_observatory_migrations_readme() {
    cat <<'MD'
# Migrations

This dry run emits deterministic Postgres migration seeds for `orgs`, `projects`,
`runs`, and `events` so the tenant hierarchy and event ingestion contract can be
reviewed before live database startup.
MD
}

render_observatory_operator_docs() {
    cat <<'MD'
# Local Observatory Operations

The dry-run scaffold reserves paths for local SaaS-ready development: API, web,
shared packages, migrations, docs, and `docker-compose.yml` with Postgres. It
prints the API-key model, scoped route list, core migrations, and progress
snapshot contract for review; no services are started.
MD
}


render_observatory_api_key_model() {
    cat <<'TS'
export type ApiKeyScope =
  | "events:write"
  | "events:read"
  | "projects:read"
  | "projects:write"
  | "runs:read"
  | "costs:read"
  | "admin:keys";

export interface ObservatoryApiKey {
  key_id: string;
  name: string;
  owner_org_id: string;
  allowed_project_ids: string[];
  allowed_repo_patterns: string[];
  allowed_event_scopes: ApiKeyScope[];
  privacy_tier_limit: "metadata-only" | "summary" | "evidence" | "full-debug";
  key_hash: string;
  created_by: string;
  created_at: string;
  last_seen_at: string | null;
  revoked_at: string | null;
}

export const OBSERVATORY_API_KEY_SCOPES: readonly ApiKeyScope[] = [
  "events:write",
  "events:read",
  "projects:read",
  "projects:write",
  "runs:read",
  "costs:read",
  "admin:keys",
] as const;

// API keys are stored hashed, shown once at creation, and scoped to one org.
// Auth failures are written as security events for audit and abuse detection.
TS
}

render_observatory_routes() {
    cat <<'TS'
export const OBSERVATORY_ROUTES = [
  "POST /v1/events",
  "POST /v1/events/batch",
  "GET /v1/projects",
  "GET /v1/projects/resolve?repo=OWNER/REPO",
  "GET /v1/projects/:id/runs",
  "GET /v1/runs/:id/progress",
  "GET /v1/runs/:id/timeline",
  "GET /v1/runs/:id/events?after_event_id=...",
  "GET /v1/runs/:id/costs",
  "GET /v1/fleet/summary",
  "GET /v1/workers?status=active",
  "GET /v1/policies/effective?project_id=...",
  "POST /v1/api-keys",
  "DELETE /v1/api-keys/:id",
] as const;

export interface EventIngestResult {
  accepted: number;
  duplicates_ignored: number;
  repeated_sequences_flagged: number;
  sequence_gaps_flagged: number;
  late_events_stored: number;
}

export async function handleEventIngest(event: ObservatoryEvent): Promise<EventIngestResult> {
  return handleEventBatchIngest({ events: [event] });
}

export async function handleEventBatchIngest(batch: ObservatoryEventBatch): Promise<EventIngestResult> {
  validateObservatoryEventBatch(batch);
  // Duplicate event_id is ignored.
  // sequence is monotonic per run_id; repeated sequence values are stored once and flagged.
  // Sequence gaps are exposed for UI review.
  // Late events are stored by occurred_at and received_at.
  return ingestValidatedEvents(batch.events);
}

export interface RunProgressSnapshot {
  run_id: string;
  status: "queued" | "running" | "blocked" | "complete" | "failed";
  progress_percent: number;
  phase: string;
  current_item: {
    title: string;
    url: string | null;
  } | null;
  queue_ready: number;
  queue_claimed: number;
  queue_blocked: number;
  queue_remaining: number;
  elapsed_ms: number;
  current_item_elapsed_ms: number;
  eta_ms: number | null;
  planned_next_step: string | null;
  last_event_id: string | null;
}

// Progress reads require runs:read and enforce owner_org_id/project boundaries.
TS
}

render_observatory_orgs_migration() {
    cat <<'SQL'
CREATE TABLE orgs (
  id TEXT PRIMARY KEY,
  slug TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
SQL
}

render_observatory_projects_migration() {
    cat <<'SQL'
CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  owner_org_id TEXT NOT NULL REFERENCES orgs(id),
  slug TEXT NOT NULL,
  display_name TEXT NOT NULL,
  privacy_tier TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (owner_org_id, slug)
);
SQL
}

render_observatory_runs_migration() {
    cat <<'SQL'
CREATE TABLE runs (
  id TEXT PRIMARY KEY,
  owner_org_id TEXT NOT NULL REFERENCES orgs(id),
  project_id TEXT NOT NULL REFERENCES projects(id),
  repo_full_name TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  completed_at TIMESTAMPTZ
);
SQL
}

render_observatory_events_migration() {
    cat <<'SQL'
CREATE TABLE events (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL UNIQUE,
  owner_org_id TEXT NOT NULL REFERENCES orgs(id),
  project_id TEXT NOT NULL REFERENCES projects(id),
  run_id TEXT REFERENCES runs(id),
  sequence INTEGER NOT NULL,
  event_scope TEXT NOT NULL,
  privacy_tier TEXT NOT NULL,
  event_payload JSONB NOT NULL,
  occurred_at TIMESTAMPTZ NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  duplicate_ignored BOOLEAN NOT NULL DEFAULT false,
  repeated_sequence BOOLEAN NOT NULL DEFAULT false,
  sequence_gap_after INTEGER,
  UNIQUE (run_id, sequence)
);
SQL
}

render_observatory_file_templates() {
    observatory_repo="$1"

    render_file_header "$observatory_repo" "docker-compose.yml"
    render_observatory_compose
    render_file_header "$observatory_repo" "apps/api/README.md"
    render_observatory_api_readme
    render_file_header "$observatory_repo" "apps/api/src/auth/api-keys.ts"
    render_observatory_api_key_model
    render_file_header "$observatory_repo" "apps/api/src/routes.ts"
    render_observatory_routes
    render_file_header "$observatory_repo" "apps/api/src/ingest/events.ts"
    render_observatory_event_ingestion_contract
    render_file_header "$observatory_repo" "apps/web/README.md"
    render_observatory_web_readme
    render_file_header "$observatory_repo" "apps/web/src/App.tsx"
    render_observatory_web_app_shell
    render_file_header "$observatory_repo" "packages/event-schema/README.md"
    render_observatory_event_schema_readme
    render_file_header "$observatory_repo" "packages/event-schema/src/events.ts"
    render_observatory_event_schema
    render_file_header "$observatory_repo" "packages/db/README.md"
    render_observatory_db_readme
    render_file_header "$observatory_repo" "packages/ui/README.md"
    render_observatory_ui_readme
    render_file_header "$observatory_repo" "migrations/README.md"
    render_observatory_migrations_readme
    render_file_header "$observatory_repo" "migrations/001_create_orgs.sql"
    render_observatory_orgs_migration
    render_file_header "$observatory_repo" "migrations/002_create_projects.sql"
    render_observatory_projects_migration
    render_file_header "$observatory_repo" "migrations/003_create_runs.sql"
    render_observatory_runs_migration
    render_file_header "$observatory_repo" "migrations/004_create_events.sql"
    render_observatory_events_migration
    render_file_header "$observatory_repo" "docs/local-operations.md"
    render_observatory_operator_docs
}

render_observatory_dry_run() {
    observatory_repo="$1"

    cat <<EOF_OBSERVATORY

${observatory_repo}/
  ${observatory_repo}/apps/
  ${observatory_repo}/apps/api/
  ${observatory_repo}/apps/api/src/
  ${observatory_repo}/apps/api/src/auth/
  ${observatory_repo}/apps/api/src/ingest/
  ${observatory_repo}/apps/web/
  ${observatory_repo}/apps/web/src/
  ${observatory_repo}/packages/
  ${observatory_repo}/packages/event-schema/
  ${observatory_repo}/packages/event-schema/src/
  ${observatory_repo}/packages/db/
  ${observatory_repo}/packages/ui/
  ${observatory_repo}/migrations/
  ${observatory_repo}/docs/
  ${observatory_repo}/docker-compose.yml
EOF_OBSERVATORY

    render_observatory_file_templates "$observatory_repo"
}

render_control_plane_dry_run() {
    owner="$1"
    governance_repo="$2"
    observatory_repo="$3"

    cat <<EOF_RENDER
# autospec-control-plane bootstrap --dry-run

owner: ${owner}
governance_repo: ${governance_repo}
observatory_repo: ${observatory_repo}
mode: dry-run
github_writes: false

${governance_repo}/
EOF_RENDER

    print_group "policies" \
        "open-source-maintainer-default.yml" \
        "private-personal-default.yml" \
        "private-company-default.yml" \
        "client-project-default.yml" \
        "research-default.yml" \
        "sandbox-default.yml"
    print_group "rules" \
        "qa.yml" "testing.yml" "documentation.yml" "security.yml" \
        "accessibility.yml" "performance.yml" "skill-generation.yml" \
        "release-readiness.yml"
    print_group "schemas" \
        "policy.schema.json" "rule.schema.json" \
        "project-class.schema.json" "priority.schema.json"
    print_group "fixtures" \
        "projects/open-source-cli.yml" "projects/private-personal-app.yml" \
        "projects/private-company-saas.yml" "projects/client-webapp.yml" \
        "projects/research-notebook.yml" "projects/sandbox-lab.yml"
    print_group "tests" \
        "policy-schema.bats" "priority-resolution.bats" "privacy-tier.bats" \
        "merge-rules.bats" "project-classification.bats" "cost-limits.bats" \
        "evidence-requirements.bats"
    print_group "docs" \
        "policy-authoring.md" "project-classes.md" "priority-waterfall.md"
    render_governance_file_templates "$governance_repo"
    render_observatory_dry_run "$observatory_repo"
}
