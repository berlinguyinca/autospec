//! §9 user re-steering classification (issue #3851; spec §9, §35,
//! §36 stage 3, §39, §50).
//!
//! Deterministic first (spec §4.1): [`looks_like_resteering`] picks
//! candidate `user_message` events out of `session_events` *before* any
//! model call. Only those candidates — redacted through
//! [`super::redact::Redactor`] (spec §39: only redacted text reaches the
//! model) — are dispatched to an [`super::Enricher`] under the
//! [`STAGE`] pipeline stage. The model output is validated against the
//! 16 §9 categories and the 1..=5 severity scale before anything is
//! written.
//!
//! Evidence (spec §35): every written `user_interventions` row reuses the
//! source event's `seq`, so the row's `(session_id, seq)` primary key is
//! itself the message reference into `session_events`; the `excerpt` JSON
//! repeats the [`Intervention::message_ref`] explicitly.
//!
//! Failure (spec §50): a classifier error records nothing — no
//! `user_interventions` row is written, the error is returned, and the
//! deterministic rows (`session_events`, `session_summaries`) stay
//! readable.

use serde::{Deserialize, Serialize};
use sqlx::{AnyPool, Row};

use crate::error::AutospecError;
use crate::insights::enrich::redact::Redactor;
use crate::insights::enrich::{Enricher, Enrichment, EnrichmentBatch};

/// Spec §36 pipeline stage for this classifier (stage 3: local failure /
/// intervention extraction).
pub const STAGE: &str = "intervention_classification";

/// The 16 user re-steering categories from spec §9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionCategory {
    Architecture,
    Scope,
    ToolSelection,
    ContextSelection,
    TestProcess,
    CodeQuality,
    ModelSelection,
    RequirementsMisunderstanding,
    RepositoryNavigation,
    UnnecessaryWork,
    Hallucination,
    Style,
    Documentation,
    Security,
    Performance,
    Other,
}

impl InterventionCategory {
    /// All 16 §9 categories, in spec §9 order.
    pub const ALL: [Self; 16] = [
        Self::Architecture,
        Self::Scope,
        Self::ToolSelection,
        Self::ContextSelection,
        Self::TestProcess,
        Self::CodeQuality,
        Self::ModelSelection,
        Self::RequirementsMisunderstanding,
        Self::RepositoryNavigation,
        Self::UnnecessaryWork,
        Self::Hallucination,
        Self::Style,
        Self::Documentation,
        Self::Security,
        Self::Performance,
        Self::Other,
    ];

    /// The spec §9 name (snake_case).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::Scope => "scope",
            Self::ToolSelection => "tool_selection",
            Self::ContextSelection => "context_selection",
            Self::TestProcess => "test_process",
            Self::CodeQuality => "code_quality",
            Self::ModelSelection => "model_selection",
            Self::RequirementsMisunderstanding => "requirements_misunderstanding",
            Self::RepositoryNavigation => "repository_navigation",
            Self::UnnecessaryWork => "unnecessary_work",
            Self::Hallucination => "hallucination",
            Self::Style => "style",
            Self::Documentation => "documentation",
            Self::Security => "security",
            Self::Performance => "performance",
            Self::Other => "other",
        }
    }

    /// Parse one §9 category name. Unrecognised names are an
    /// [`AutospecError`] so a mis-labelled model output fails closed
    /// instead of mis-bucketing.
    pub fn parse(name: &str) -> Result<Self, AutospecError> {
        Self::ALL
            .iter()
            .find(|category| category.as_str() == name)
            .copied()
            .ok_or_else(|| {
                AutospecError::parse(
                    "intervention_category",
                    format!("unrecognised intervention category {name:?}"),
                )
            })
    }
}

/// §9 severity scale bounds: an integer from 1 to 5.
pub const SEVERITY_MIN: i64 = 1;
pub const SEVERITY_MAX: i64 = 5;

/// True when `severity` sits on the §9 1..=5 scale.
pub fn valid_severity(severity: i64) -> bool {
    (SEVERITY_MIN..=SEVERITY_MAX).contains(&severity)
}

/// One classified user re-steering intervention (spec §9 record shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intervention {
    pub session_id: String,
    pub category: InterventionCategory,
    /// Integer 1..=5 per spec §9.
    pub severity: i64,
    /// Names the source `session_events` row: `<session_id>:<seq>`.
    pub message_ref: String,
    pub semantic_summary: String,
    pub resolved_in_session: bool,
}

/// The `user_interventions.excerpt` document: the redacted source quote
/// plus the classification, so the row stays traceable to its source
/// message (§35) without re-running the classifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Excerpt {
    /// The redacted source quote — only redacted text is stored (§39).
    quote: String,
    /// §9 severity, stored as an integer 1..=5.
    severity: i64,
    semantic_summary: String,
    resolved_in_session: bool,
    message_ref: String,
}

/// Deterministic re-steering markers, drawn from the spec §9 examples:
/// imperatives ("Don't …", "Stop …"), corrections ("use the existing …",
/// "we already have …", "wrong …"), and challenge questions ("why are you
/// …"). A `user_message` payload containing any marker (case
/// insensitively) is a candidate for the model; everything else is
/// filtered out deterministically before any model call (spec §4.1).
const RESTEERING_MARKERS: [&str; 12] = [
    "don't",
    "do not",
    "dont",
    "stop",
    "use the existing",
    "already have",
    "we already",
    "wrong",
    "why are you",
    "run the tests first",
    "that's not what",
    "i meant",
];

/// The deterministic pre-filter: true when the message looks like user
/// re-steering.
pub fn looks_like_resteering(text: &str) -> bool {
    let lowered = text.to_lowercase();
    RESTEERING_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
}

/// One candidate `user_message` event picked by the pre-filter.
struct Candidate {
    seq: i64,
    text: String,
}

/// One model classification, parsed from [`Enrichment::value`] (JSON).
/// The enricher returns one record per candidate;
/// [`Enrichment::index`] is the 0-based candidate index.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Classification {
    category: String,
    severity: i64,
    #[serde(default)]
    semantic_summary: String,
    #[serde(default)]
    resolved_in_session: bool,
}

fn map_error(operation: &str, error: sqlx::Error) -> AutospecError {
    AutospecError::state("user_interventions", format!("{operation}: {error}"))
}

/// Portable DDL (SQLite + Postgres, ADR 0001 D10) for the tables this
/// classifier reads and writes. `CREATE TABLE IF NOT EXISTS` is a no-op
/// where the insights migration (`3000001_init.sql`) already created
/// them.
const CLASSIFY_DDL: &str = "CREATE TABLE IF NOT EXISTS session_events (\
    session_id TEXT NOT NULL, \
    seq INTEGER NOT NULL, \
    event_type TEXT NOT NULL, \
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
    payload TEXT, \
    PRIMARY KEY (session_id, seq) \
); CREATE TABLE IF NOT EXISTS user_interventions (\
    session_id TEXT NOT NULL, \
    seq INTEGER NOT NULL, \
    intervention_type TEXT NOT NULL, \
    occurred_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP, \
    excerpt TEXT, \
    PRIMARY KEY (session_id, seq) \
)";

async fn ensure_schema(pool: &AnyPool) -> Result<(), AutospecError> {
    for statement in CLASSIFY_DDL.split(';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        sqlx::query(statement)
            .execute(pool)
            .await
            .map_err(|error| map_error("ensure schema", error))?;
    }
    Ok(())
}

/// Read the session's `user_message` events and keep the ones the
/// deterministic pre-filter flags as re-steering.
async fn candidate_events(
    pool: &AnyPool,
    session_id: &str,
) -> Result<Vec<Candidate>, AutospecError> {
    let rows = sqlx::query(
        "SELECT seq, payload FROM session_events \
         WHERE session_id = $1 AND event_type = 'user_message' \
         AND payload IS NOT NULL AND TRIM(payload) <> '' \
         ORDER BY seq",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(|error| map_error("read session_events", error))?;
    let mut candidates = Vec::new();
    for row in rows {
        let seq = row
            .try_get::<i64, _>(0)
            .map_err(|error| map_error("decode seq", error))?;
        let text = row
            .try_get::<String, _>(1)
            .map_err(|error| map_error("decode payload", error))?;
        if looks_like_resteering(&text) {
            candidates.push(Candidate { seq, text });
        }
    }
    Ok(candidates)
}

/// Validate one [`Enrichment`] against the §9 contract and map it to the
/// source candidate.
fn classify_one(
    result: &Enrichment,
    candidates: &[Candidate],
    session_id: &str,
) -> Result<(Intervention, Excerpt), AutospecError> {
    let candidate = candidates.get(result.index as usize).ok_or_else(|| {
        AutospecError::validation(format!(
            "classifier returned index {} for {} candidate(s)",
            result.index,
            candidates.len()
        ))
    })?;
    let classification: Classification = serde_json::from_str(&result.value)
        .map_err(|error| AutospecError::parse("intervention classification", error.to_string()))?;
    let category = InterventionCategory::parse(&classification.category)?;
    if !valid_severity(classification.severity) {
        return Err(AutospecError::validation(format!(
            "classifier severity {} is outside the §9 scale 1..=5",
            classification.severity
        )));
    }
    let message_ref = format!("{session_id}:{}", candidate.seq);
    let quote = Redactor::redact(&candidate.text);
    Ok((
        Intervention {
            session_id: session_id.to_string(),
            category,
            severity: classification.severity,
            message_ref: message_ref.clone(),
            semantic_summary: classification.semantic_summary.clone(),
            resolved_in_session: classification.resolved_in_session,
        },
        Excerpt {
            quote,
            severity: classification.severity,
            semantic_summary: classification.semantic_summary,
            resolved_in_session: classification.resolved_in_session,
            message_ref,
        },
    ))
}

/// Classify the §9 user re-steering interventions of one session.
///
/// Reads the session's `user_message` events, applies the deterministic
/// pre-filter, dispatches the redacted candidates to `enricher` (stage
/// [`STAGE`]), validates the model output against the 16 categories and
/// the 1..=5 severity scale, and writes one `user_interventions` row per
/// detected intervention (keyed on the source event's `seq`, so each row
/// cites its source message — §35).
///
/// A classifier failure records nothing: no row is written and the
/// error is returned, leaving the deterministic rows (`session_events`,
/// `session_summaries`) readable.
pub async fn classify(
    pool: &AnyPool,
    enricher: &dyn Enricher,
    session_id: &str,
) -> Result<Vec<Intervention>, AutospecError> {
    ensure_schema(pool).await?;
    let candidates = candidate_events(pool, session_id).await?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let items = candidates
        .iter()
        .map(|candidate| Redactor::redact(&candidate.text))
        .collect();
    let batch = EnrichmentBatch {
        session_id: session_id.to_string(),
        stage: STAGE.to_string(),
        repo: None,
        cursor: 0,
        items,
    };

    // The model call is the only fallible step, and it runs before any
    // write: a classifier error records nothing (spec §50).
    let results = enricher.enrich(&batch).map_err(|error| {
        AutospecError::state(
            "user_interventions",
            format!("classifier failed for session {session_id}: {error}"),
        )
    })?;

    let mut rows: Vec<(Intervention, Excerpt)> = Vec::with_capacity(results.len());
    for result in &results {
        rows.push(classify_one(result, &candidates, session_id)?);
    }
    for (intervention, excerpt) in &rows {
        let excerpt_json = serde_json::to_string(excerpt)
            .map_err(|error| AutospecError::parse("intervention excerpt", error.to_string()))?;
        sqlx::query(
            "INSERT INTO user_interventions (session_id, seq, intervention_type, excerpt) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (session_id, seq) DO UPDATE SET \
             intervention_type = excluded.intervention_type, \
             excerpt = excluded.excerpt",
        )
        .bind(&intervention.session_id)
        .bind(
            intervention
                .message_ref
                .rsplit_once(':')
                .map(|(_, seq)| seq)
                .unwrap_or_default()
                .parse::<i64>()
                .map_err(|error| AutospecError::parse("message_ref seq", error.to_string()))?,
        )
        .bind(intervention.category.as_str())
        .bind(&excerpt_json)
        .execute(pool)
        .await
        .map_err(|error| map_error("insert intervention", error))?;
    }
    Ok(rows
        .into_iter()
        .map(|(intervention, _)| intervention)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::summarize::{store_summary, SessionSummary};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Test database URL: `AUTOSPEC_TEST_DB_URL` (disposable PostgreSQL 16
    /// under Apptainer in the operator full run) when set; otherwise a
    /// disposable per-test SQLite file. Always a real database — no
    /// database mocks.
    fn test_db_url() -> String {
        match std::env::var("AUTOSPEC_TEST_DB_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => format!(
                "sqlite://{}/autospec-classify-{}-{}.db",
                std::env::temp_dir().display(),
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ),
        }
    }

    async fn open_test_pool() -> AnyPool {
        let pool = crate::resources::db::open_shared_db(&test_db_url())
            .await
            .expect("test database must open");
        ensure_schema(&pool).await.expect("ddl");
        pool
    }

    fn unique(prefix: &str) -> String {
        format!(
            "{prefix}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        )
    }

    /// Seed `session_events` rows: one per (event_type, payload) pair, in
    /// order, with seq = 0, 1, 2, …
    async fn seed_events(pool: &AnyPool, session_id: &str, events: &[(&str, &str)]) {
        for (seq, (event_type, payload)) in events.iter().enumerate() {
            sqlx::query(
                "INSERT INTO session_events (session_id, seq, event_type, payload) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(session_id)
            .bind(seq as i64)
            .bind(event_type)
            .bind(payload)
            .execute(pool)
            .await
            .expect("seed event");
        }
    }

    /// Seed a `session_summaries` row through the deterministic §8 store
    /// path, so the "summary stays readable" assertions read a row a
    /// production path wrote.
    async fn seed_summary(pool: &AnyPool, session_id: &str, interventions: u64) {
        let summary = SessionSummary {
            session_id: session_id.to_string(),
            task_type: None,
            task_domain: Vec::new(),
            outcome: None,
            autonomy_score: 1.0,
            user_interventions: interventions,
            review_rework_count: 0,
            tool_calls: 2,
            tool_errors: 0,
            files_read: 0,
            files_changed: 0,
            tests_run: 0,
            context_peak_tokens: 0,
            input_tokens: 0,
            output_tokens: 0,
            estimated_cost: 0.0,
            duration_seconds: 0,
            models: Vec::new(),
            commits: Vec::new(),
            pull_requests: Vec::new(),
        };
        store_summary(pool, &summary).await.expect("summary row");
    }

    /// Canned category per §9 example (local fixtures): the stub
    /// "model" reads the redacted item and returns the category its
    /// wording asks for.
    fn fixture_classification(item: &str) -> (&'static str, i64) {
        let lowered = item.to_lowercase();
        if lowered.contains("abstraction") {
            ("architecture", 3)
        } else if lowered.contains("existing service") {
            ("scope", 2)
        } else if lowered.contains("getters and setters") {
            ("code_quality", 2)
        } else if lowered.contains("run the tests first") {
            ("test_process", 3)
        } else if lowered.contains("wrong directory") {
            ("repository_navigation", 2)
        } else if lowered.contains("helper") {
            ("unnecessary_work", 2)
        } else if lowered.contains("that model") {
            ("model_selection", 2)
        } else if lowered.contains("git history") {
            ("tool_selection", 2)
        } else {
            ("other", 1)
        }
    }

    /// Stub enricher over the local fixtures: classifies each candidate
    /// by [`fixture_classification`] and records every batch so tests can
    /// assert exactly what reached the model.
    struct StubEnricher {
        batches: Mutex<Vec<EnrichmentBatch>>,
        fail: Mutex<bool>,
    }

    impl StubEnricher {
        fn new() -> Self {
            Self {
                batches: Mutex::new(Vec::new()),
                fail: Mutex::new(false),
            }
        }
        fn failing(self) -> Self {
            *self.fail.lock().unwrap() = true;
            self
        }
    }

    impl Enricher for StubEnricher {
        fn enrich(&self, batch: &EnrichmentBatch) -> Result<Vec<Enrichment>, AutospecError> {
            self.batches.lock().unwrap().push(batch.clone());
            if *self.fail.lock().unwrap() {
                return Err(AutospecError::other("injected classifier failure"));
            }
            Ok(batch
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let (category, severity) = fixture_classification(item);
                    Enrichment {
                        session_id: batch.session_id.clone(),
                        stage: batch.stage.clone(),
                        index: i as u64,
                        kind: "intervention".into(),
                        value: serde_json::json!({
                            "category": category,
                            "severity": severity,
                            "semantic_summary": format!("stub summary {i}"),
                            "resolved_in_session": i == 0,
                        })
                        .to_string(),
                        model: "stub".into(),
                    }
                })
                .collect())
        }
    }

    async fn intervention_rows(pool: &AnyPool, session_id: &str) -> Vec<(i64, String, String)> {
        sqlx::raw_sql(
            format!(
                "SELECT seq, intervention_type, excerpt FROM user_interventions \
                 WHERE session_id = '{session_id}' ORDER BY seq"
            )
            .as_str(),
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.try_get::<i64, _>(0).unwrap(),
                row.try_get::<String, _>(1).unwrap(),
                row.try_get::<String, _>(2).unwrap(),
            )
        })
        .collect()
    }

    /// TDD category-coverage test (lands first): the §9 taxonomy is
    /// exactly 16 categories, each round-trips name <-> variant, and the
    /// severity scale is the §9 1..=5 integers.
    #[test]
    fn all_sixteen_section_nine_categories_round_trip() {
        assert_eq!(InterventionCategory::ALL.len(), 16);
        assert_eq!(
            InterventionCategory::ALL
                .iter()
                .map(|category| category.as_str())
                .collect::<Vec<_>>(),
            vec![
                "architecture",
                "scope",
                "tool_selection",
                "context_selection",
                "test_process",
                "code_quality",
                "model_selection",
                "requirements_misunderstanding",
                "repository_navigation",
                "unnecessary_work",
                "hallucination",
                "style",
                "documentation",
                "security",
                "performance",
                "other",
            ]
        );
        for category in InterventionCategory::ALL {
            assert_eq!(
                InterventionCategory::parse(category.as_str()).unwrap(),
                category
            );
            assert_eq!(
                serde_json::to_value(category).unwrap(),
                serde_json::json!(category.as_str())
            );
        }
        assert!(InterventionCategory::parse("vibes").is_err());
        for severity in -1i64..=6 {
            assert_eq!(valid_severity(severity), (1..=5).contains(&severity));
        }
    }

    /// The deterministic pre-filter flags every §9 example and rejects
    /// ordinary steering-free user messages.
    #[test]
    fn pre_filter_flags_every_section_nine_example() {
        for example in [
            "Don't create another abstraction.",
            "Use the existing service.",
            "Stop adding getters and setters.",
            "Run the tests first.",
            "Don't rewrite this.",
            "You're looking in the wrong directory.",
            "We already have a helper for this.",
            "Don't use that model.",
            "Why are you searching git history?",
        ] {
            assert!(looks_like_resteering(example), "missed: {example:?}");
        }
        for ordinary in [
            "please continue",
            "looks great, ship it",
            "add a unit test for the parser",
            "what is the current status?",
        ] {
            assert!(
                !looks_like_resteering(ordinary),
                "false positive: {ordinary:?}"
            );
        }
        // Case-insensitive: the markers are lowercased, not the words.
        assert!(looks_like_resteering("STOP adding getters and setters."));
    }

    /// classify writes 1 `user_interventions` row per detected
    /// intervention, each citing its source `session_events` row, and the
    /// pre-filter kept the non-candidate message away from the model.
    #[tokio::test]
    async fn classify_writes_one_row_per_intervention_citing_its_message() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique("cls");
        seed_events(
            &pool,
            &session_id,
            &[
                ("user_message", "Don't create another abstraction."),
                ("user_message", "please continue"),
                (
                    "user_message",
                    "Stop adding getters and setters. Run the tests first.",
                ),
            ],
        )
        .await;

        let enricher = StubEnricher::new();
        let interventions = classify(&pool, &enricher, &session_id)
            .await
            .expect("classify");

        assert_eq!(interventions.len(), 2);
        assert_eq!(
            interventions[0].category,
            InterventionCategory::Architecture
        );
        assert_eq!(interventions[0].severity, 3);
        assert_eq!(interventions[0].message_ref, format!("{session_id}:0"));
        assert_eq!(interventions[1].category, InterventionCategory::CodeQuality);
        assert_eq!(interventions[1].severity, 2);
        assert_eq!(interventions[1].message_ref, format!("{session_id}:2"));

        // Only the two pre-filter candidates reached the model.
        let batches = enricher.batches.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].stage, STAGE);
        assert_eq!(batches[0].items.len(), 2);
        assert!(!batches[0]
            .items
            .iter()
            .any(|item| item.contains("please continue")));

        // One row per intervention, keyed on the source event's seq.
        let rows = intervention_rows(&pool, &session_id).await;
        assert_eq!(
            rows,
            vec![
                (0, "architecture".to_string(), rows[0].2.clone()),
                (2, "code_quality".to_string(), rows[1].2.clone()),
            ]
        );
        // §35: every message_ref names a real session_events row.
        let joined = sqlx::raw_sql(
            format!(
                "SELECT COUNT(*) FROM user_interventions ui \
                 JOIN session_events se ON se.session_id = ui.session_id AND se.seq = ui.seq \
                 WHERE ui.session_id = '{session_id}'"
            )
            .as_str(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(joined.try_get::<i64, _>(0).unwrap(), 2);

        // The excerpt carries the classification with severity as an
        // integer and the explicit message_ref.
        let excerpt: Excerpt = serde_json::from_str(&rows[0].2).expect("excerpt json");
        assert_eq!(excerpt.severity, 3);
        assert_eq!(excerpt.message_ref, format!("{session_id}:0"));
        assert_eq!(excerpt.quote, "Don't create another abstraction.");
    }

    /// Only redacted text reaches the model, and the stored quote is
    /// redacted too (spec §39).
    #[tokio::test]
    async fn classified_quote_is_stored_redacted() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique("redact");
        let secret = "AKIAIOSFODNN7EXAMPLE"; // linter:allow-SECURITY AWS doc example key, non-secret fixture
        seed_events(
            &pool,
            &session_id,
            &[(
                "user_message",
                &format!("Don't use {secret} in the test suite."),
            )],
        )
        .await;

        let enricher = StubEnricher::new();
        classify(&pool, &enricher, &session_id)
            .await
            .expect("classify");

        let batches = enricher.batches.lock().unwrap();
        let dispatched = &batches[0].items[0];
        assert!(
            !dispatched.contains(secret),
            "secret leaked to model: {dispatched}"
        );
        assert!(
            dispatched.contains("[REDACTED:aws-access-key]"),
            "{dispatched}"
        );

        let rows = intervention_rows(&pool, &session_id).await;
        assert_eq!(rows.len(), 1);
        let excerpt: Excerpt = serde_json::from_str(&rows[0].2).expect("excerpt json");
        assert!(
            !excerpt.quote.contains(secret),
            "secret stored: {}",
            excerpt.quote
        );
        assert!(
            excerpt.quote.contains("[REDACTED:aws-access-key]"),
            "{}",
            excerpt.quote
        );
    }

    /// A classifier failure records nothing and returns Err; the
    /// deterministic `session_summaries` row stays readable.
    #[tokio::test]
    async fn classifier_failure_records_nothing_and_summary_stays_readable() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique("fail");
        seed_events(
            &pool,
            &session_id,
            &[(
                "user_message",
                "Stop adding getters and setters. Run the tests first.",
            )],
        )
        .await;
        seed_summary(&pool, &session_id, 2).await;

        let enricher = StubEnricher::new().failing();
        assert!(
            classify(&pool, &enricher, &session_id).await.is_err(),
            "a classifier failure must return Err"
        );
        assert!(
            intervention_rows(&pool, &session_id).await.is_empty(),
            "a classifier failure records nothing"
        );

        let row = sqlx::raw_sql(
            format!(
                "SELECT user_interventions FROM session_summaries WHERE session_id = '{session_id}'"
            )
            .as_str(),
        )
        .fetch_one(&pool)
        .await
        .expect("session_summaries row must stay readable");
        assert_eq!(row.try_get::<i64, _>(0).unwrap(), 2);
    }

    /// A session with no re-steering candidates records nothing and the
    /// model is never called.
    #[tokio::test]
    async fn session_without_candidates_records_nothing() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique("none");
        seed_events(
            &pool,
            &session_id,
            &[
                ("user_message", "please continue"),
                ("tool_call", "stop the daemon"),
            ],
        )
        .await;

        let enricher = StubEnricher::new();
        let interventions = classify(&pool, &enricher, &session_id)
            .await
            .expect("classify");
        assert!(interventions.is_empty());
        assert!(
            enricher.batches.lock().unwrap().is_empty(),
            "no candidates means no model call"
        );
        assert!(intervention_rows(&pool, &session_id).await.is_empty());
    }

    /// Re-classifying the same session is idempotent: one row per source
    /// event, never a duplicate.
    #[tokio::test]
    async fn classify_is_idempotent_per_source_event() {
        let _guard = ENV_LOCK.lock().unwrap();
        let pool = open_test_pool().await;
        let session_id = unique("idem");
        seed_events(
            &pool,
            &session_id,
            &[
                ("user_message", "Don't create another abstraction."),
                ("user_message", "Stop adding getters and setters."),
            ],
        )
        .await;

        let enricher = StubEnricher::new();
        classify(&pool, &enricher, &session_id)
            .await
            .expect("first pass");
        classify(&pool, &enricher, &session_id)
            .await
            .expect("second pass");
        assert_eq!(intervention_rows(&pool, &session_id).await.len(), 2);
    }
}
