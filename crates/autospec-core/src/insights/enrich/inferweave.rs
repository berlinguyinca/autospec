//! Local InferWeave enrichment backend (issue #3848, spec §37).
//!
//! [`InferWeaveEnricher`] is the [`super::Enricher`] implementation for the
//! InferWeave semantic-service family. It is a **low-priority** client:
//! every request is stamped `priority: "low"` so the local model pool can
//! pre-empt enrichment work with interactive inference (spec §37
//! "low-priority, resumable jobs; local models preferred").
//!
//! Policy gate: when `semantic_analysis.remote_allowed` is `false` the
//! backend is confined to a local endpoint (loopback / localhost / Unix
//! socket) and refuses to dispatch anywhere else — fail-closed, before
//! any bytes leave the process.

use std::sync::Arc;

use crate::error::AutospecError;
use crate::insights::config::SemanticAnalysisConfig;
use crate::insights::enrich::{Enricher, Enrichment, EnrichmentBatch};

/// One low-priority dispatch to the InferWeave endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferWeaveRequest {
    pub endpoint: String,
    /// Model name from `semantic_analysis.model`.
    pub model: String,
    /// Spec §37: enrichment is low-priority background work.
    pub priority: &'static str,
    /// Pipeline stage this batch belongs to (spec §36 8-stage pipeline).
    pub stage: String,
    /// Redacted payload items (the queue guarantees these passed
    /// `Redactor::redact` — see the redaction gate in [`super::queue`]).
    pub items: Vec<String>,
}

/// Transport seam: the wire call to the InferWeave endpoint. The policy
/// gates live in [`InferWeaveEnricher`]; the transport only executes an
/// already-authorized request.
pub trait InferWeaveSender: Send + Sync {
    fn send(&self, request: &InferWeaveRequest) -> Result<Vec<Enrichment>, AutospecError>;
}

/// Local InferWeave [`Enricher`] honoring the `semantic_analysis` config.
pub struct InferWeaveEnricher {
    endpoint: String,
    config: SemanticAnalysisConfig,
    sender: Arc<dyn InferWeaveSender>,
}

impl InferWeaveEnricher {
    /// Build the backend. Fails closed on a non-InferWeave provider or an
    /// empty endpoint.
    pub fn new(
        endpoint: impl Into<String>,
        config: SemanticAnalysisConfig,
        sender: Arc<dyn InferWeaveSender>,
    ) -> Result<Self, AutospecError> {
        let endpoint = endpoint.into();
        if config.provider != "inferweave" {
            return Err(AutospecError::validation(format!(
                "inferweave backend requires semantic_analysis.provider \"inferweave\", got {config:?}"
            )));
        }
        if endpoint.trim().is_empty() {
            return Err(AutospecError::validation(
                "inferweave backend requires an endpoint",
            ));
        }
        Ok(Self {
            endpoint,
            config,
            sender,
        })
    }

    /// True when `endpoint` points at the local machine: loopback
    /// (`127.0.0.1`, `::1`, `localhost`) or a Unix socket (`unix://`).
    pub fn endpoint_is_local(endpoint: &str) -> bool {
        let (scheme, rest) = match endpoint.split_once("://") {
            Some((scheme, rest)) => (Some(scheme), rest),
            None => (None, endpoint),
        };
        if scheme == Some("unix") {
            return true;
        }
        // A bracketed host is IPv6 ("[::1]:8787"); split on the closing
        // bracket, not on ':' which appears inside the address.
        let host = if let Some(bracketed) = rest.strip_prefix('[') {
            bracketed.split(']').next().unwrap_or("")
        } else {
            rest.split(['/', ':']).next().unwrap_or("")
        };
        matches!(host, "127.0.0.1" | "localhost" | "::1")
    }
}

impl Enricher for InferWeaveEnricher {
    fn enrich(&self, batch: &EnrichmentBatch) -> Result<Vec<Enrichment>, AutospecError> {
        if !self.config.remote_allowed && !Self::endpoint_is_local(&self.endpoint) {
            return Err(AutospecError::validation(format!(
                "remote endpoint {} is not allowed: semantic_analysis.remote_allowed is false \
                 (session {} stays unenriched)",
                self.endpoint, batch.session_id
            )));
        }
        let request = InferWeaveRequest {
            endpoint: self.endpoint.clone(),
            model: self.config.model.clone(),
            priority: "low",
            stage: batch.stage.clone(),
            items: batch.items.clone(),
        };
        self.sender.send(&request)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Records every request it is handed; optional failure injection for
    /// queue tests.
    pub struct RecordingSender {
        pub requests: std::sync::Mutex<Vec<InferWeaveRequest>>,
        pub fail_after: std::sync::Mutex<Option<usize>>,
        pub enrichments_per_batch: usize,
    }

    impl RecordingSender {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                requests: std::sync::Mutex::new(Vec::new()),
                fail_after: std::sync::Mutex::new(None),
                enrichments_per_batch: 1,
            })
        }
    }

    impl InferWeaveSender for RecordingSender {
        fn send(&self, request: &InferWeaveRequest) -> Result<Vec<Enrichment>, AutospecError> {
            self.requests.lock().unwrap().push(request.clone());
            let mut fail_after = self.fail_after.lock().unwrap();
            if let Some(limit) = *fail_after {
                if self.requests.lock().unwrap().len() > limit {
                    return Err(AutospecError::other("injected enricher failure"));
                }
                *fail_after = Some(limit + 1);
            }
            let base = request.items.first().cloned().unwrap_or_default();
            Ok((0..self.enrichments_per_batch.min(request.items.len()))
                .map(|i| Enrichment {
                    session_id: String::new(),
                    stage: request.stage.clone(),
                    index: i as u64,
                    kind: "stub".into(),
                    value: format!("{base}#{i}"),
                    model: request.model.clone(),
                })
                .collect())
        }
    }

    fn local_config() -> SemanticAnalysisConfig {
        SemanticAnalysisConfig {
            provider: "inferweave".into(),
            model: "qwen3-semantic".into(),
            remote_allowed: false,
        }
    }

    fn batch(items: &[&str]) -> EnrichmentBatch {
        EnrichmentBatch {
            session_id: "s1".into(),
            stage: "embedding".into(),
            repo: Some("acme/web".into()),
            cursor: 0,
            items: items.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn remote_allowed_false_confines_backend_to_local_endpoint() {
        let sender = RecordingSender::new();
        let enricher = InferWeaveEnricher::new(
            "https://api.inferweave.ai/v1",
            local_config(),
            sender.clone(),
        )
        .unwrap();
        let err = enricher
            .enrich(&batch(&["hello"]))
            .expect_err("remote endpoint must be refused");
        assert!(err.to_string().contains("remote_allowed"), "{}", err);
        assert!(
            sender.requests.lock().unwrap().is_empty(),
            "no bytes may leave the process"
        );
    }

    #[test]
    fn local_endpoint_is_allowed_and_low_priority() {
        let sender = RecordingSender::new();
        let enricher =
            InferWeaveEnricher::new("http://127.0.0.1:8787/v1", local_config(), sender.clone())
                .unwrap();
        let out = enricher.enrich(&batch(&["hello"])).unwrap();
        assert_eq!(out.len(), 1);
        let req = &sender.requests.lock().unwrap()[0];
        assert_eq!(req.priority, "low");
        assert_eq!(req.model, "qwen3-semantic");
        assert_eq!(req.items, vec!["hello".to_string()]);
    }

    #[test]
    fn remote_endpoint_allowed_when_remote_allowed_is_true() {
        let sender = RecordingSender::new();
        let mut config = local_config();
        config.remote_allowed = true;
        let enricher =
            InferWeaveEnricher::new("https://api.inferweave.ai/v1", config, sender.clone())
                .unwrap();
        assert!(enricher.enrich(&batch(&["hello"])).is_ok());
        assert_eq!(sender.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn constructor_rejects_wrong_provider_and_empty_endpoint() {
        let sender = RecordingSender::new();
        let mut config = local_config();
        config.provider = "openai".into();
        assert!(InferWeaveEnricher::new("http://127.0.0.1", config, sender.clone()).is_err());
        assert!(InferWeaveEnricher::new("  ", local_config(), sender).is_err());
    }

    #[test]
    fn endpoint_locality_classification() {
        assert!(InferWeaveEnricher::endpoint_is_local(
            "http://127.0.0.1:8787/v1"
        ));
        assert!(InferWeaveEnricher::endpoint_is_local(
            "http://localhost:8787"
        ));
        assert!(InferWeaveEnricher::endpoint_is_local("http://[::1]:8787"));
        assert!(InferWeaveEnricher::endpoint_is_local(
            "unix:///tmp/inferweave.sock"
        ));
        assert!(!InferWeaveEnricher::endpoint_is_local(
            "https://api.inferweave.ai/v1"
        ));
        assert!(!InferWeaveEnricher::endpoint_is_local(
            "http://10.0.0.5:8787"
        ));
    }
}
