use serde::Serialize;

use super::config::FallbackConfig;
use super::error::CodeIntelError;
use super::schema::{Operation, ResultSource};

/// One step of the degradation ladder, and why the gateway took it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FallbackStep {
    pub from: String,
    pub to: String,
    pub reason: String,
}

/// The resolved degradation ladder: `lsp -> ast-grep -> ripgrep`.
///
/// The chain is computed per operation and per configuration. An operation with
/// no lower-confidence approximation (hover, type hierarchy, diagnostics) has a
/// single-entry chain and fails closed instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackChain {
    operation: Operation,
    sources: Vec<ResultSource>,
}

impl FallbackChain {
    pub fn resolve(operation: Operation, config: &FallbackConfig) -> Self {
        let mut sources = vec![ResultSource::Lsp];
        if config.structural.is_some() && operation.has_structural_fallback() {
            sources.push(ResultSource::AstGrep);
        }
        if config.textual.is_some() && operation.has_textual_fallback() {
            sources.push(ResultSource::Ripgrep);
        }
        Self { operation, sources }
    }

    pub fn sources(&self) -> &[ResultSource] {
        &self.sources
    }

    /// The next source to try after `current` failed, if any.
    pub fn next_after(&self, current: ResultSource) -> Option<ResultSource> {
        let position = self.sources.iter().position(|source| *source == current)?;
        self.sources.get(position + 1).copied()
    }

    pub fn has_degraded_path(&self) -> bool {
        self.sources.len() > 1
    }

    /// The tool binary backing a source, for the doctor report.
    pub fn tool_for(&self, source: ResultSource, config: &FallbackConfig) -> Option<String> {
        match source {
            ResultSource::Lsp => None,
            ResultSource::AstGrep => config.structural.clone(),
            ResultSource::Ripgrep => config.textual.clone(),
        }
    }

    /// Degrade after a failure.
    ///
    /// `require_semantic` is the caller's assertion that a lower-confidence
    /// answer would be worse than no answer — an impact gate, for instance,
    /// cannot be satisfied by a textual guess. Non-degradable failures (a bad
    /// config, a rejected gate) never degrade regardless.
    pub fn degrade(
        &self,
        current: ResultSource,
        error: &CodeIntelError,
        require_semantic: bool,
    ) -> Result<(ResultSource, FallbackStep), CodeIntelError> {
        if require_semantic {
            return Err(CodeIntelError::new(
                error.kind(),
                format!(
                    "{} requires semantic certainty and cannot fall back: {}",
                    self.operation.as_api_name(),
                    error.message()
                ),
            ));
        }
        if !error.is_degradable() {
            return Err(error.clone());
        }
        let next = self.next_after(current).ok_or_else(|| {
            CodeIntelError::new(
                error.kind(),
                format!(
                    "{} exhausted its fallback chain: {}",
                    self.operation.as_api_name(),
                    error.message()
                ),
            )
        })?;
        let step = FallbackStep {
            from: current.as_str().to_string(),
            to: next.as_str().to_string(),
            reason: error.to_string(),
        };
        Ok((next, step))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> FallbackConfig {
        FallbackConfig::default()
    }

    #[test]
    fn references_degrade_through_the_full_ladder() {
        let chain = FallbackChain::resolve(Operation::References, &config());

        assert_eq!(
            chain.sources(),
            &[
                ResultSource::Lsp,
                ResultSource::AstGrep,
                ResultSource::Ripgrep
            ]
        );
    }

    #[test]
    fn type_dependent_operations_have_no_degraded_path() {
        for operation in [
            Operation::Hover,
            Operation::Diagnostics,
            Operation::TypeHierarchy,
        ] {
            let chain = FallbackChain::resolve(operation, &config());

            assert_eq!(chain.sources(), &[ResultSource::Lsp]);
            assert!(!chain.has_degraded_path());
        }
    }

    #[test]
    fn implementations_stop_at_the_structural_tier() {
        let chain = FallbackChain::resolve(Operation::Implementations, &config());

        assert_eq!(chain.sources(), &[ResultSource::Lsp, ResultSource::AstGrep]);
    }

    #[test]
    fn a_disabled_structural_tool_removes_that_rung() {
        let config = FallbackConfig {
            structural: None,
            textual: Some("rg".to_string()),
        };

        let chain = FallbackChain::resolve(Operation::References, &config);

        assert_eq!(chain.sources(), &[ResultSource::Lsp, ResultSource::Ripgrep]);
    }

    #[test]
    fn a_backend_failure_degrades_one_rung_and_records_why() {
        let chain = FallbackChain::resolve(Operation::References, &config());
        let error = CodeIntelError::backend("rust-analyzer exited");

        let (source, step) = chain.degrade(ResultSource::Lsp, &error, false).unwrap();

        assert_eq!(source, ResultSource::AstGrep);
        assert_eq!(step.from, "lsp");
        assert_eq!(step.to, "ast-grep");
        assert!(step.reason.contains("rust-analyzer exited"));
    }

    #[test]
    fn requiring_semantic_certainty_refuses_to_degrade() {
        let chain = FallbackChain::resolve(Operation::Impact, &config());
        let error = CodeIntelError::timeout("index not ready");

        let refusal = chain.degrade(ResultSource::Lsp, &error, true).unwrap_err();

        assert!(refusal.message().contains("requires semantic certainty"));
    }

    #[test]
    fn a_config_error_never_degrades() {
        let chain = FallbackChain::resolve(Operation::References, &config());
        let error = CodeIntelError::config("unknown key in backend: mode2");

        let refusal = chain.degrade(ResultSource::Lsp, &error, false).unwrap_err();

        assert_eq!(refusal, error);
    }

    #[test]
    fn exhausting_the_chain_surfaces_the_original_failure() {
        let chain = FallbackChain::resolve(Operation::References, &config());
        let error = CodeIntelError::backend("ripgrep missing");

        let refusal = chain
            .degrade(ResultSource::Ripgrep, &error, false)
            .unwrap_err();

        assert!(refusal.message().contains("exhausted its fallback chain"));
        assert!(refusal.message().contains("ripgrep missing"));
    }

    #[test]
    fn tools_are_named_for_the_doctor_report() {
        let config = config();
        let chain = FallbackChain::resolve(Operation::References, &config);

        assert_eq!(chain.tool_for(ResultSource::Lsp, &config), None);
        assert_eq!(
            chain.tool_for(ResultSource::AstGrep, &config).as_deref(),
            Some("ast-grep")
        );
        assert_eq!(
            chain.tool_for(ResultSource::Ripgrep, &config).as_deref(),
            Some("rg")
        );
    }
}
