-- Subsystem: insights (continuous improvement engine, spec
-- docs/specs/2026-09-08-continuous-improvement-engine.md §34).
-- Version range 3xxxxxx (3000001-3999999) is the development range owned
-- by the insights subsystem; no other subsystem may emit a migration whose
-- version is in this range (D7 shared migration protocol).
--
-- One portable DDL file applies unchanged on PostgreSQL 16 and SQLite (D10):
-- only portable types (TEXT, INTEGER, REAL, BOOLEAN, TIMESTAMP) and
-- CREATE TABLE/INDEX IF NOT EXISTS are used.
--
-- Privacy (§39): raw prompts MAY contain secrets, so payload/summary/excerpt
-- text columns are stored but NEVER indexed; every index is on a short
-- structural identifier sized for §51's 1M events/day.
--
-- Evidence (§35): finding_evidence references are non-nullable so every
-- finding stays traceable to a session (and, where applicable, an event,
-- commit, PR, or CI run).

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    work_item_id TEXT,
    harness TEXT,
    model TEXT,
    status TEXT NOT NULL,
    started_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ended_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sessions_repo ON sessions (repo);
CREATE INDEX IF NOT EXISTS idx_sessions_work_item ON sessions (work_item_id);

CREATE TABLE IF NOT EXISTS session_events (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Raw prompt/message text; deliberately not indexed (§39).
    payload TEXT,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_session_events_occurred_at ON session_events (occurred_at);

CREATE TABLE IF NOT EXISTS session_summaries (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    -- Generated summary text; deliberately not indexed (§39).
    summary TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (session_id)
);

CREATE TABLE IF NOT EXISTS user_interventions (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    intervention_type TEXT NOT NULL,
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Intervention excerpt text; deliberately not indexed (§39).
    excerpt TEXT,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_user_interventions_occurred_at ON user_interventions (occurred_at);

CREATE TABLE IF NOT EXISTS tool_invocations (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    tool_name TEXT NOT NULL,
    args_summary TEXT,
    status TEXT NOT NULL,
    duration_ms INTEGER,
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_tool_invocations_tool_name ON tool_invocations (tool_name);

CREATE TABLE IF NOT EXISTS model_invocations (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    model TEXT NOT NULL,
    tokens_in INTEGER,
    tokens_out INTEGER,
    latency_ms INTEGER,
    status TEXT NOT NULL,
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_model_invocations_model ON model_invocations (model);

CREATE TABLE IF NOT EXISTS git_events (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    repo TEXT NOT NULL,
    commit_sha TEXT,
    event_type TEXT NOT NULL,
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Structured git payload (JSON); deliberately not indexed (§39).
    payload TEXT,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_git_events_repo ON git_events (repo);
CREATE INDEX IF NOT EXISTS idx_git_events_commit_sha ON git_events (commit_sha);

CREATE TABLE IF NOT EXISTS ci_events (
    session_id TEXT NOT NULL REFERENCES sessions (id),
    seq INTEGER NOT NULL,
    repo TEXT NOT NULL,
    run_id TEXT NOT NULL,
    status TEXT NOT NULL,
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Structured CI payload (JSON); deliberately not indexed (§39).
    payload TEXT,
    PRIMARY KEY (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_ci_events_repo ON ci_events (repo);
CREATE INDEX IF NOT EXISTS idx_ci_events_run_id ON ci_events (run_id);

CREATE TABLE IF NOT EXISTS review_findings (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    work_item_id TEXT,
    repo TEXT,
    category TEXT NOT NULL,
    severity TEXT,
    title TEXT NOT NULL,
    -- Finding description text; deliberately not indexed (§39).
    description TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_review_findings_session_id ON review_findings (session_id);
CREATE INDEX IF NOT EXISTS idx_review_findings_work_item ON review_findings (work_item_id);
CREATE INDEX IF NOT EXISTS idx_review_findings_repo ON review_findings (repo);

CREATE TABLE IF NOT EXISTS quality_findings (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions (id),
    work_item_id TEXT,
    repo TEXT,
    gate TEXT NOT NULL,
    severity TEXT,
    title TEXT NOT NULL,
    -- Finding description text; deliberately not indexed (§39).
    description TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_quality_findings_session_id ON quality_findings (session_id);
CREATE INDEX IF NOT EXISTS idx_quality_findings_work_item ON quality_findings (work_item_id);
CREATE INDEX IF NOT EXISTS idx_quality_findings_repo ON quality_findings (repo);

CREATE TABLE IF NOT EXISTS patterns (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    title TEXT NOT NULL,
    -- Pattern description text; deliberately not indexed (§39).
    description TEXT,
    session_count INTEGER NOT NULL DEFAULT 0,
    first_seen_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    status TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_patterns_kind ON patterns (kind);

-- §35 evidence preservation: every finding must be traceable back to
-- evidence; the references below are non-nullable for exactly that reason.
CREATE TABLE IF NOT EXISTS finding_evidence (
    id TEXT PRIMARY KEY,
    finding_id TEXT NOT NULL,
    evidence_kind TEXT NOT NULL,
    session_id TEXT NOT NULL,
    event_ref TEXT,
    message_ref TEXT,
    commit_sha TEXT,
    pr_ref TEXT,
    ci_run_ref TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_finding_evidence_finding_id ON finding_evidence (finding_id);
CREATE INDEX IF NOT EXISTS idx_finding_evidence_session_id ON finding_evidence (session_id);

CREATE TABLE IF NOT EXISTS improvement_proposals (
    id TEXT PRIMARY KEY,
    finding_id TEXT NOT NULL,
    title TEXT NOT NULL,
    -- Proposed instruction/skill body; deliberately not indexed (§39).
    body TEXT,
    target TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_improvement_proposals_finding_id ON improvement_proposals (finding_id);
CREATE INDEX IF NOT EXISTS idx_improvement_proposals_status ON improvement_proposals (status);

CREATE TABLE IF NOT EXISTS proposal_evaluations (
    proposal_id TEXT NOT NULL REFERENCES improvement_proposals (id),
    seq INTEGER NOT NULL,
    evaluator TEXT NOT NULL,
    verdict TEXT NOT NULL,
    -- Evaluation rationale text; deliberately not indexed (§39).
    rationale TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (proposal_id, seq)
);

CREATE TABLE IF NOT EXISTS configuration_versions (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    -- Configuration content; deliberately not indexed (§39).
    content TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_configuration_versions_name ON configuration_versions (name);

CREATE TABLE IF NOT EXISTS post_change_measurements (
    id TEXT PRIMARY KEY,
    proposal_id TEXT,
    repo TEXT,
    work_item_id TEXT,
    metric TEXT NOT NULL,
    before_value REAL,
    after_value REAL,
    measured_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_post_change_measurements_proposal_id ON post_change_measurements (proposal_id);
CREATE INDEX IF NOT EXISTS idx_post_change_measurements_work_item ON post_change_measurements (work_item_id);
