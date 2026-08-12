use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewRisk {
    Normal,
    High,
    Integration,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewReasoning {
    Standard,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReviewPolicyInput {
    pub changed_paths: Vec<String>,
    pub serialization_reasons: Vec<String>,
    pub logical_component_count: usize,
    pub has_producer_surface: bool,
    pub has_consumer_surface: bool,
    pub critical_boundary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRequirements {
    pub risk: ReviewRisk,
    pub reviewer_reasoning: ReviewReasoning,
    pub integration_shaped: bool,
    pub require_integration_smoke: bool,
    pub prefer_provider_diversity: bool,
    pub require_provider_diversity: bool,
    pub reasons: Vec<String>,
}

pub fn classify_review_requirements(input: &ReviewPolicyInput) -> ReviewRequirements {
    let mut risk = ReviewRisk::Normal;
    let mut reasons = BTreeSet::new();
    let mut integration_shaped = false;

    for path in &input.changed_paths {
        let path = normalize_path(path);
        for (reason, patterns) in PATH_CATEGORIES {
            if patterns.iter().any(|pattern| path.contains(pattern)) {
                reasons.insert((*reason).to_string());
                risk = risk.max(ReviewRisk::Integration);
                integration_shaped = true;
            }
        }
    }

    if input.logical_component_count >= 2 {
        reasons.insert("logical-components:multiple".to_string());
        risk = risk.max(ReviewRisk::Integration);
        integration_shaped = true;
    }

    if input.has_producer_surface && input.has_consumer_surface {
        reasons.insert("boundary:producer-consumer".to_string());
        risk = risk.max(ReviewRisk::Integration);
        integration_shaped = true;
    }

    for reason in &input.serialization_reasons {
        match reason.as_str() {
            "priority:high" => {
                reasons.insert("issue:priority-high".to_string());
                risk = risk.max(ReviewRisk::High);
                integration_shaped = true;
            }
            "reasoning:deep" => {
                reasons.insert("issue:reasoning-deep".to_string());
                risk = risk.max(ReviewRisk::High);
                integration_shaped = true;
            }
            _ => {}
        }
    }

    if input.critical_boundary {
        reasons.insert("boundary:critical".to_string());
        risk = ReviewRisk::Critical;
        integration_shaped = true;
    }

    let prefer_provider_diversity = risk >= ReviewRisk::High;

    ReviewRequirements {
        risk,
        reviewer_reasoning: if risk == ReviewRisk::Normal {
            ReviewReasoning::Standard
        } else {
            ReviewReasoning::High
        },
        integration_shaped,
        require_integration_smoke: integration_shaped,
        prefer_provider_diversity,
        require_provider_diversity: risk == ReviewRisk::Critical,
        reasons: reasons.into_iter().collect(),
    }
}

const PATH_CATEGORIES: &[(&str, &[&str])] = &[
    (
        "path:orchestration",
        &["orchestrat", "autonomous", "conductor"],
    ),
    (
        "path:install-bootstrap",
        &["install.sh", "bootstrap.sh", "/install/", "/bootstrap/"],
    ),
    (
        "path:adapter-provider",
        &["/adapter", "/provider", "/codex/", "/opencode/"],
    ),
    ("path:daemon-session", &["daemon", "session"]),
    ("path:state-recovery", &["/state/", "recovery", "resume"]),
    (
        "path:merge-claim-premerge-authority",
        &["merge", "claim", "premerge"],
    ),
];

fn normalize_path(path: &str) -> String {
    path.trim_start_matches("./")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        changed_paths: &[&str],
        serialization_reasons: &[&str],
        logical_component_count: usize,
        critical_boundary: bool,
    ) -> ReviewPolicyInput {
        ReviewPolicyInput {
            changed_paths: changed_paths
                .iter()
                .map(|path| (*path).to_string())
                .collect(),
            serialization_reasons: serialization_reasons
                .iter()
                .map(|reason| (*reason).to_string())
                .collect(),
            logical_component_count,
            critical_boundary,
            ..ReviewPolicyInput::default()
        }
    }

    fn requirements(
        risk: ReviewRisk,
        reviewer_reasoning: ReviewReasoning,
        integration_shaped: bool,
        prefer_provider_diversity: bool,
        require_provider_diversity: bool,
        reasons: &[&str],
    ) -> ReviewRequirements {
        ReviewRequirements {
            risk,
            reviewer_reasoning,
            integration_shaped,
            require_integration_smoke: integration_shaped,
            prefer_provider_diversity,
            require_provider_diversity,
            reasons: reasons.iter().map(|reason| (*reason).to_string()).collect(),
        }
    }

    type Case = (&'static str, ReviewPolicyInput, ReviewRequirements);

    fn docs_case() -> Case {
        let input = input(&["docs/USER_MANUAL.md"], &[], 0, false);
        let expected = requirements(
            ReviewRisk::Normal,
            ReviewReasoning::Standard,
            false,
            false,
            false,
            &[],
        );
        ("one docs path", input, expected)
    }

    fn install_case() -> Case {
        let input = input(&["scripts/install.sh"], &[], 0, false);
        let expected = requirements(
            ReviewRisk::Integration,
            ReviewReasoning::High,
            true,
            true,
            false,
            &["path:install-bootstrap"],
        );
        ("install surface", input, expected)
    }

    fn daemon_adapter_case() -> Case {
        let paths = [
            "scripts/autospec-daemon.sh",
            "skills/autospec-run/codex/prompt.md",
            "scripts/autospec-daemon.sh",
        ];
        let input = input(&paths, &[], 0, false);
        let expected = requirements(
            ReviewRisk::Integration,
            ReviewReasoning::High,
            true,
            true,
            false,
            &["path:adapter-provider", "path:daemon-session"],
        );
        ("daemon and adapter surfaces", input, expected)
    }

    fn priority_case() -> Case {
        let input = input(&[], &["priority:high"], 0, false);
        let expected = requirements(
            ReviewRisk::High,
            ReviewReasoning::High,
            true,
            true,
            false,
            &["issue:priority-high"],
        );
        ("priority high", input, expected)
    }

    fn deep_components_case() -> Case {
        let input = input(&[], &["reasoning:deep", "reasoning:deep"], 2, false);
        let expected = requirements(
            ReviewRisk::Integration,
            ReviewReasoning::High,
            true,
            true,
            false,
            &["issue:reasoning-deep", "logical-components:multiple"],
        );
        ("deep reasoning and two logical components", input, expected)
    }

    fn critical_case() -> Case {
        let input = input(&["docs/USER_MANUAL.md"], &[], 0, true);
        let expected = requirements(
            ReviewRisk::Critical,
            ReviewReasoning::High,
            true,
            true,
            true,
            &["boundary:critical"],
        );
        (
            "critical boundary overrides weaker signals",
            input,
            expected,
        )
    }

    #[test]
    fn review_requirements_classify_repository_risk_shapes() {
        let cases = [
            docs_case(),
            install_case(),
            daemon_adapter_case(),
            priority_case(),
            deep_components_case(),
            critical_case(),
        ];

        for (name, input, expected) in cases {
            assert_eq!(classify_review_requirements(&input), expected, "{}", name);
        }
    }
}
