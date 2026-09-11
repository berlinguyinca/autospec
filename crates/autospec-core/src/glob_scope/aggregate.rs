//! Counting a set of patterns without counting a member twice.
//!
//! The fleet report said 6 workers / 24 slots. There were 5 workers and 20
//! slots. The inventory was built by matching each worker against a list of
//! name patterns and summing, and `qwen3.8-27b-vision-1` matched both
//! `qwen3.8-27b-*` and `qwen3.8-27b-vision-*`, so it contributed its 4 slots
//! twice. Nothing in the output distinguished "summed over patterns" from
//! "distinct things": both were integers in the same sentence.
//!
//! [`aggregate`] keeps the two counts separate and names the double-counted
//! members, and [`Aggregate::report_line`] renders the line that cannot be
//! misread — distinct first, summed second, overlap named.

use std::collections::BTreeMap;

use super::glob::Glob;

/// One pattern and the members it matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternCount {
    pub pattern: String,
    pub matched: Vec<String>,
    pub weight: u64,
}

/// A member counted by more than one pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoubleCount {
    pub member: String,
    pub patterns: Vec<String>,
    pub weight: u64,
}

/// The result of matching a weighted inventory against a set of patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate {
    per_pattern: Vec<PatternCount>,
    distinct_weight: u64,
    summed_weight: u64,
    double_counted: Vec<DoubleCount>,
}

impl Aggregate {
    /// Per-pattern match counts, in the order the patterns were given.
    pub fn per_pattern(&self) -> &[PatternCount] {
        &self.per_pattern
    }

    /// Members matched by more than one pattern, with every pattern that took
    /// them. Non-empty means the summed figures are wrong.
    pub fn double_counted(&self) -> &[DoubleCount] {
        &self.double_counted
    }

    /// True when any member was matched by two or more patterns.
    pub fn is_double_counted(&self) -> bool {
        !self.double_counted.is_empty()
    }

    /// Distinct members: each counted once however many patterns took it.
    pub fn distinct_count(&self) -> usize {
        self.per_pattern
            .iter()
            .flat_map(|p| p.matched.iter().cloned())
            .collect::<BTreeMap<String, ()>>()
            .len()
    }

    /// The count you get by summing per-pattern matches — the wrong one,
    /// whenever [`Aggregate::is_double_counted`].
    pub fn summed_count(&self) -> usize {
        self.per_pattern.iter().map(|p| p.matched.len()).sum()
    }

    /// Distinct members' weight (slots, GPUs, dollars).
    pub fn distinct_weight(&self) -> u64 {
        self.distinct_weight
    }

    /// Weight summed over patterns.
    pub fn summed_weight(&self) -> u64 {
        self.summed_weight
    }

    /// The report line: distinct first, summed second, overlap named. A line
    /// that states only one number is how 24 got reported as the fleet size.
    pub fn report_line(&self) -> String {
        let base = format!(
            "{} distinct / {} weight (summed over {} pattern(s): {} / {})",
            self.distinct_count(),
            self.distinct_weight,
            self.per_pattern.len(),
            self.summed_count(),
            self.summed_weight,
        );
        if self.double_counted.is_empty() {
            return base;
        }
        let overlap = self
            .double_counted
            .iter()
            .map(|d| format!("{} <- {}", d.member, d.patterns.join(" + ")))
            .collect::<Vec<_>>()
            .join("; ");
        format!("{} — double-counted: {}", base, overlap)
    }
}

/// Match `inventory` (name, weight) against `patterns` and keep the summed and
/// distinct counts apart.
///
/// A member matched by no pattern is not counted at all — that is a different
/// defect (an unlabelled member) than double counting, and the caller who owns
/// the inventory should see it in their own enumeration, not here.
pub fn aggregate(inventory: &[(String, u64)], patterns: &[Glob]) -> Aggregate {
    let mut per_pattern = Vec::with_capacity(patterns.len());
    let mut hit_patterns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut weight_of: BTreeMap<String, u64> = BTreeMap::new();

    for pattern in patterns {
        let mut matched = Vec::new();
        let mut weight = 0;
        for (name, member_weight) in inventory {
            if pattern.matches(name) {
                matched.push(name.clone());
                weight += member_weight;
                hit_patterns
                    .entry(name.clone())
                    .or_default()
                    .push(pattern.pattern().to_owned());
                weight_of.insert(name.clone(), *member_weight);
            }
        }
        per_pattern.push(PatternCount {
            pattern: pattern.pattern().to_owned(),
            matched,
            weight,
        });
    }

    let distinct_weight = weight_of.values().sum();
    let summed_weight = per_pattern.iter().map(|p| p.weight).sum();
    let double_counted = hit_patterns
        .into_iter()
        .filter(|(_, hits)| hits.len() > 1)
        .map(|(member, patterns)| DoubleCount {
            member: member.clone(),
            patterns,
            weight: weight_of.get(&member).copied().unwrap_or(0),
        })
        .collect();

    Aggregate {
        per_pattern,
        distinct_weight,
        summed_weight,
        double_counted,
    }
}
