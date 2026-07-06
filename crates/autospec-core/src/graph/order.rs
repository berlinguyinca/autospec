use crate::spec::SpecMetadata;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphErrorKind {
    MissingDependency,
    Cycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphError {
    pub kind: GraphErrorKind,
    pub spec_id: Option<String>,
    pub dependency: Option<String>,
    pub cycle: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Visited,
}

pub fn execution_order(specs: &[SpecMetadata]) -> Result<Vec<String>, GraphError> {
    let by_id = specs
        .iter()
        .map(|spec| (spec.id.as_str().to_string(), spec))
        .collect::<BTreeMap<_, _>>();

    for spec in specs {
        for dependency in &spec.dependencies {
            if !by_id.contains_key(dependency) {
                return Err(GraphError {
                    kind: GraphErrorKind::MissingDependency,
                    spec_id: Some(spec.id.as_str().to_string()),
                    dependency: Some(dependency.clone()),
                    cycle: Vec::new(),
                    message: format!("{} depends on missing {dependency}", spec.id.as_str()),
                });
            }
        }
    }

    let mut states = BTreeMap::new();
    let mut order = Vec::new();
    let mut stack = Vec::new();
    let roots = stable_roots(specs);

    for id in roots {
        visit(&id, &by_id, &mut states, &mut stack, &mut order)?;
    }

    Ok(order)
}

fn stable_roots(specs: &[SpecMetadata]) -> Vec<String> {
    let mut ids = specs
        .iter()
        .map(|spec| {
            (
                spec.version
                    .as_str()
                    .trim_start_matches('V')
                    .parse::<u64>()
                    .unwrap_or(u64::MAX),
                spec.id.as_str().to_string(),
            )
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids.into_iter().map(|(_, id)| id).collect()
}

fn visit(
    id: &str,
    by_id: &BTreeMap<String, &SpecMetadata>,
    states: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
    order: &mut Vec<String>,
) -> Result<(), GraphError> {
    match states.get(id) {
        Some(VisitState::Visited) => return Ok(()),
        Some(VisitState::Visiting) => return Err(cycle_error(id, stack)),
        None => {}
    }

    states.insert(id.to_string(), VisitState::Visiting);
    stack.push(id.to_string());

    let spec = by_id.get(id).expect("dependencies were prevalidated");
    let dependencies = spec.dependencies.iter().cloned().collect::<BTreeSet<_>>();
    for dependency in dependencies {
        visit(&dependency, by_id, states, stack, order)?;
    }

    stack.pop();
    states.insert(id.to_string(), VisitState::Visited);
    order.push(id.to_string());
    Ok(())
}

fn cycle_error(id: &str, stack: &[String]) -> GraphError {
    let start = stack
        .iter()
        .position(|candidate| candidate == id)
        .unwrap_or(0);
    let mut cycle = stack[start..].to_vec();
    cycle.push(id.to_string());
    GraphError {
        kind: GraphErrorKind::Cycle,
        spec_id: Some(id.to_string()),
        dependency: None,
        message: format!("dependency cycle detected: {}", cycle.join(" -> ")),
        cycle,
    }
}
