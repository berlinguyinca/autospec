use super::{ManagedProjectError, ManagedProjectPolicy, RemoteProject, PROJECT_FETCH_LIMIT};
use autospec_core::managed_project::{ItemKey, ManagedProjectIdentity};
use serde_json::Value;
use std::collections::HashSet;
use std::ops::Range;

const MARKER_BEGIN: &str = "<!-- autospec-managed-project:begin -->";
const MARKER_END: &str = "<!-- autospec-managed-project:end -->";
const MAX_CAPSULE_BYTES: usize = 32 * 1024;
const MAX_CAPSULE_ITEMS: usize = 200;
const MAX_ITEM_EDGES: usize = 200;
const MAX_ITEM_BYTES: usize = 2 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioRecoveryCapsule {
    schema: String,
    portfolio_id: String,
    plan_digest: String,
    create_nonce: String,
    items: Vec<PortfolioRecoveryItem>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PortfolioRecoveryItem {
    item_key: String,
    repository: String,
    role: String,
    completion_policy: String,
    local_parents: Vec<String>,
    dependencies: Vec<String>,
}

impl PortfolioRecoveryCapsule {
    pub fn from_value(value: &Value) -> Result<Self, ManagedProjectError> {
        let bytes = serde_json::to_vec(value).map_err(json_error)?;
        if bytes.len() > MAX_CAPSULE_BYTES {
            return Err(ManagedProjectError::new(
                "portfolio recovery capsule exceeds the serialized byte limit",
            ));
        }
        Self::parse_value(value)
    }

    fn from_json(bytes: &[u8]) -> Result<Self, ManagedProjectError> {
        if bytes.len() > MAX_CAPSULE_BYTES {
            return Err(ManagedProjectError::new(
                "portfolio recovery capsule exceeds the serialized byte limit",
            ));
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|error| {
            ManagedProjectError::new(format!("invalid portfolio recovery capsule: {error}"))
        })?;
        Self::parse_value(&value)
    }

    pub fn to_value(&self) -> Result<Value, ManagedProjectError> {
        Ok(serde_json::json!({
            "schema": self.schema,
            "portfolio_id": self.portfolio_id,
            "plan_digest": self.plan_digest,
            "create_nonce": self.create_nonce,
            "items": self.items.iter().map(|item| serde_json::json!({
                "item_key": item.item_key,
                "repository": item.repository,
                "role": item.role,
                "completion_policy": item.completion_policy,
                "local_parents": item.local_parents,
                "dependencies": item.dependencies,
            })).collect::<Vec<_>>(),
        }))
    }

    fn to_json(&self) -> Result<String, ManagedProjectError> {
        let json = serde_json::to_string(&self.to_value()?).map_err(json_error)?;
        if json.len() > MAX_CAPSULE_BYTES {
            return Err(ManagedProjectError::new(
                "portfolio recovery capsule exceeds the serialized byte limit",
            ));
        }
        Ok(json)
    }

    pub fn portfolio_id(&self) -> &str {
        &self.portfolio_id
    }

    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }

    pub fn create_nonce(&self) -> &str {
        &self.create_nonce
    }

    fn parse_value(value: &Value) -> Result<Self, ManagedProjectError> {
        let object = exact_object(
            value,
            &[
                "schema",
                "portfolio_id",
                "plan_digest",
                "create_nonce",
                "items",
            ],
            "portfolio recovery capsule",
        )?;
        let items = object
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ManagedProjectError::new("portfolio recovery capsule items must be an array")
            })?
            .iter()
            .map(|item| {
                let item = exact_object(
                    item,
                    &[
                        "item_key",
                        "repository",
                        "role",
                        "completion_policy",
                        "local_parents",
                        "dependencies",
                    ],
                    "portfolio recovery capsule item",
                )?;
                Ok(PortfolioRecoveryItem {
                    item_key: required_string(item, "item_key", "portfolio recovery capsule item")?,
                    repository: required_string(
                        item,
                        "repository",
                        "portfolio recovery capsule item",
                    )?,
                    role: required_string(item, "role", "portfolio recovery capsule item")?,
                    completion_policy: required_string(
                        item,
                        "completion_policy",
                        "portfolio recovery capsule item",
                    )?,
                    local_parents: required_string_array(item, "local_parents")?,
                    dependencies: required_string_array(item, "dependencies")?,
                })
            })
            .collect::<Result<Vec<_>, ManagedProjectError>>()?;
        let capsule = Self {
            schema: required_string(object, "schema", "portfolio recovery capsule")?,
            portfolio_id: required_string(object, "portfolio_id", "portfolio recovery capsule")?,
            plan_digest: required_string(object, "plan_digest", "portfolio recovery capsule")?,
            create_nonce: required_string(object, "create_nonce", "portfolio recovery capsule")?,
            items,
        };
        capsule.validate()?;
        Ok(capsule)
    }

    fn validate(&self) -> Result<(), ManagedProjectError> {
        if self.schema != "autospec.portfolio-recovery.v1" {
            return Err(ManagedProjectError::new(
                "unsupported portfolio recovery capsule schema",
            ));
        }
        require_lower_hex(&self.portfolio_id, 64, "portfolio id")?;
        require_lower_hex(&self.plan_digest, 64, "plan digest")?;
        require_lower_hex(&self.create_nonce, 32, "create nonce")?;
        if self.items.is_empty() || self.items.len() > MAX_CAPSULE_ITEMS {
            return Err(ManagedProjectError::new(
                "portfolio recovery capsule item count is outside the strict bound",
            ));
        }
        for item in &self.items {
            if serde_json::to_vec(&item.to_value())
                .map_err(json_error)?
                .len()
                > MAX_ITEM_BYTES
            {
                return Err(ManagedProjectError::new(
                    "portfolio recovery capsule item exceeds the serialized byte limit",
                ));
            }
            ItemKey::new(&item.item_key).map_err(ManagedProjectError::new)?;
            validate_repository(&item.repository)?;
            let expected_policy = match item.role.as_str() {
                "source-tracker" | "repo-tracker" => "closed-tracker",
                "prerequisite" => "external-prerequisite",
                "implementation" => "merged-pr",
                "audit" => "audit-receipt",
                _ => {
                    return Err(ManagedProjectError::new(
                        "portfolio recovery capsule has an unsupported item role",
                    ))
                }
            };
            if item.completion_policy != expected_policy {
                return Err(ManagedProjectError::new(
                    "portfolio recovery capsule item role and completion policy conflict",
                ));
            }
            for edges in [&item.local_parents, &item.dependencies] {
                if edges.len() > MAX_ITEM_EDGES {
                    return Err(ManagedProjectError::new(
                        "portfolio recovery capsule item edge count exceeds the strict bound",
                    ));
                }
                for edge in edges {
                    ItemKey::new(edge).map_err(ManagedProjectError::new)?;
                }
            }
        }
        Ok(())
    }
}

impl PortfolioRecoveryItem {
    fn to_value(&self) -> Value {
        serde_json::json!({
            "item_key": self.item_key,
            "repository": self.repository,
            "role": self.role,
            "completion_policy": self.completion_policy,
            "local_parents": self.local_parents,
            "dependencies": self.dependencies,
        })
    }
}

fn exact_object<'a>(
    value: &'a Value,
    fields: &[&str],
    context: &str,
) -> Result<&'a serde_json::Map<String, Value>, ManagedProjectError> {
    let object = value
        .as_object()
        .ok_or_else(|| ManagedProjectError::new(format!("{context} must be an object")))?;
    if object.len() != fields.len() || object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(ManagedProjectError::new(format!(
            "{context} contains missing or unknown fields"
        )));
    }
    Ok(object)
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<String, ManagedProjectError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ManagedProjectError::new(format!("{context} has invalid {field}")))
}

fn required_string_array(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Vec<String>, ManagedProjectError> {
    object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ManagedProjectError::new(format!(
                "portfolio recovery capsule item has invalid {field}"
            ))
        })?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    ManagedProjectError::new(format!(
                        "portfolio recovery capsule item has invalid {field}"
                    ))
                })
        })
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum MarkerDisposition {
    Missing,
    Exact { legacy: bool },
    Other,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ProjectCandidate {
    pub(super) number: u64,
    pub(super) title: String,
}

pub(super) fn classify_marker(
    readme: &str,
    identity: &ManagedProjectIdentity,
    owner: &str,
    recovery_capsule: Option<&PortfolioRecoveryCapsule>,
) -> Result<MarkerDisposition, ManagedProjectError> {
    let Some(marker) = parse_marker(readme)? else {
        return Ok(MarkerDisposition::Missing);
    };
    let identity_matches = match (&marker.identity, identity) {
        (
            MarkerIdentity::Product { product_key },
            ManagedProjectIdentity::Product {
                product_key: expected,
            },
        ) => product_key == expected.as_str(),
        (
            MarkerIdentity::SpecPortfolio {
                portfolio_id,
                source,
                recovery_capsule: actual_capsule,
            },
            ManagedProjectIdentity::SpecPortfolio(expected),
        ) => {
            let portfolio_matches = portfolio_id == expected.portfolio_id().as_str();
            let source_matches = source == &expected.source().to_string();
            if portfolio_matches != source_matches {
                return Err(ManagedProjectError::new(
                    "spec portfolio marker ID conflicts with its source identity",
                ));
            }
            if portfolio_matches
                && recovery_capsule.is_some_and(|expected| expected != actual_capsule)
            {
                return Err(ManagedProjectError::new(
                    "spec portfolio marker recovery capsule conflicts with local state",
                ));
            }
            portfolio_matches
        }
        _ => false,
    };
    if !identity_matches {
        return Ok(MarkerDisposition::Other);
    }
    if marker.owner != owner {
        return Err(ManagedProjectError::new(format!(
            "managed GitHub Project marker owner {} conflicts with approved owner {owner}",
            marker.owner
        )));
    }
    Ok(MarkerDisposition::Exact {
        legacy: marker.legacy,
    })
}

pub(super) fn upsert_marker(
    readme: &str,
    identity: &ManagedProjectIdentity,
    owner: &str,
    recovery_capsule: Option<&PortfolioRecoveryCapsule>,
) -> Result<String, ManagedProjectError> {
    let marker = render_marker(identity, owner, recovery_capsule)?;
    let Some(range) = marker_range(readme)? else {
        return if readme.is_empty() {
            Ok(marker)
        } else {
            Ok(format!("{readme}\n\n{marker}"))
        };
    };
    match classify_marker(readme, identity, owner, recovery_capsule)? {
        MarkerDisposition::Exact { .. } => {
            let mut updated = String::with_capacity(readme.len() + marker.len());
            updated.push_str(&readme[..range.start]);
            updated.push_str(&marker);
            updated.push_str(&readme[range.end..]);
            Ok(updated)
        }
        MarkerDisposition::Missing => unreachable!("marker range exists"),
        MarkerDisposition::Other => Err(ManagedProjectError::new(
            "GitHub Project contains a different managed marker",
        )),
    }
}

pub(super) fn parse_project_candidates(
    output: &str,
) -> Result<Vec<ProjectCandidate>, ManagedProjectError> {
    let value: Value = serde_json::from_str(output).map_err(json_error)?;
    if value.is_array() {
        return parse_graphql_project_pages(&value);
    }
    parse_legacy_project_candidates(&value)
}

fn parse_legacy_project_candidates(
    value: &Value,
) -> Result<Vec<ProjectCandidate>, ManagedProjectError> {
    let projects = value
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project list has no projects array"))?;
    let total_count = match value.get("totalCount") {
        None => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            ManagedProjectError::new("GitHub Project list totalCount must be an unsigned integer")
        })?),
    };
    if total_count.is_some_and(|count| count != projects.len() as u64)
        || (total_count.is_none() && projects.len() >= PROJECT_FETCH_LIMIT)
    {
        return Err(ManagedProjectError::new(
            "GitHub Project discovery may be truncated at the transport limit",
        ));
    }
    collect_project_candidates(projects, &mut HashSet::new())
}

fn parse_graphql_project_pages(
    value: &Value,
) -> Result<Vec<ProjectCandidate>, ManagedProjectError> {
    let pages = value.as_array().expect("checked GraphQL pages array");
    if pages.is_empty() {
        return Err(ManagedProjectError::new(
            "GitHub Project pagination returned no pages",
        ));
    }
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let connection = page
            .pointer("/data/repositoryOwner/projectsV2")
            .ok_or_else(|| {
                ManagedProjectError::new("GitHub Project page has no projectsV2 connection")
            })?;
        let nodes = connection
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project page has no nodes array"))?;
        candidates.extend(collect_project_candidates(nodes, &mut seen)?);
        let page_info = connection
            .get("pageInfo")
            .and_then(Value::as_object)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project page has no pageInfo"))?;
        let has_next = page_info
            .get("hasNextPage")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                ManagedProjectError::new("GitHub Project page has invalid hasNextPage")
            })?;
        let is_last = index + 1 == pages.len();
        if has_next == is_last {
            return Err(ManagedProjectError::new(
                "GitHub Project pagination is truncated or contains an extra page",
            ));
        }
        if has_next
            && !page_info
                .get("endCursor")
                .and_then(Value::as_str)
                .is_some_and(|cursor| !cursor.is_empty())
        {
            return Err(ManagedProjectError::new(
                "GitHub Project pagination has no continuation cursor",
            ));
        }
    }
    Ok(candidates)
}

fn collect_project_candidates(
    projects: &[Value],
    seen: &mut HashSet<u64>,
) -> Result<Vec<ProjectCandidate>, ManagedProjectError> {
    projects
        .iter()
        .map(|project| {
            let number = project
                .get("number")
                .and_then(Value::as_u64)
                .filter(|number| *number > 0)
                .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid number"))?;
            if !seen.insert(number) {
                return Err(ManagedProjectError::new(
                    "GitHub Project pagination contains a duplicate project number",
                ));
            }
            let title = project
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(ProjectCandidate { number, title })
        })
        .collect()
}

pub(super) fn verify_managed_marker(
    readme: &str,
    policy: &ManagedProjectPolicy,
) -> Result<bool, ManagedProjectError> {
    let identity = ManagedProjectIdentity::Product {
        product_key: policy.product_key.clone(),
    };
    classify_marker(readme, &identity, &policy.owner, None)
        .map(|disposition| matches!(disposition, MarkerDisposition::Exact { .. }))
}

pub(super) fn parse_project(output: &str) -> Result<RemoteProject, ManagedProjectError> {
    let value: Value = serde_json::from_str(output).map_err(json_error)?;
    let string = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| ManagedProjectError::new(format!("GitHub Project has invalid {field}")))
    };
    let owner = value
        .pointer("/owner/login")
        .or_else(|| value.get("owner"))
        .and_then(Value::as_str)
        .filter(|owner| !owner.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid owner"))?;
    let readme = value
        .get("readme")
        .and_then(Value::as_str)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid readme"))?
        .to_owned();
    Ok(RemoteProject {
        node_id: string("id")?,
        number: value
            .get("number")
            .and_then(Value::as_u64)
            .filter(|number| *number > 0)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project has invalid number"))?,
        url: string("url")?,
        title: string("title")?,
        owner,
        readme,
    })
}

pub(super) fn parse_project_items(output: &str) -> Result<HashSet<String>, ManagedProjectError> {
    let value: Value = serde_json::from_str(output).map_err(json_error)?;
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ManagedProjectError::new("GitHub Project item list has no items array"))?;
    if items.len() >= PROJECT_FETCH_LIMIT {
        return Err(ManagedProjectError::new(
            "GitHub Project item list may be truncated at the transport limit",
        ));
    }
    let mut issues = HashSet::new();
    for item in items {
        let content = item
            .get("content")
            .and_then(Value::as_object)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project item has invalid content"))?;
        let item_type = content
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| ManagedProjectError::new("GitHub Project item has invalid type"))?;
        match item_type {
            "Issue" => {
                let url = content
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ManagedProjectError::new("GitHub issue item has invalid URL"))?;
                issues.insert(normalize_issue_url(url)?);
            }
            "PullRequest" | "DraftIssue" | "RedactedItem" => {}
            _ => {
                return Err(ManagedProjectError::new(format!(
                    "GitHub Project item has unknown type {item_type}"
                )))
            }
        }
    }
    Ok(issues)
}

pub(super) fn normalize_issue_url(url: &str) -> Result<String, ManagedProjectError> {
    let normalized = url
        .trim()
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_ascii_lowercase();
    let path = normalized
        .strip_prefix("https://github.com/")
        .ok_or_else(|| ManagedProjectError::new("issue URL must use https://github.com"))?;
    let parts = path.split('/').collect::<Vec<_>>();
    let number = parts
        .get(3)
        .filter(|_| parts.len() == 4 && !parts[0].is_empty() && !parts[1].is_empty())
        .filter(|_| parts[2] == "issues")
        .and_then(|number| number.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| ManagedProjectError::new("issue URL must identify one GitHub issue"))?;
    Ok(format!(
        "https://github.com/{}/{}/issues/{number}",
        parts[0], parts[1]
    ))
}

struct Marker {
    identity: MarkerIdentity,
    owner: String,
    legacy: bool,
}

enum MarkerIdentity {
    Product {
        product_key: String,
    },
    SpecPortfolio {
        portfolio_id: String,
        source: String,
        recovery_capsule: PortfolioRecoveryCapsule,
    },
}

fn parse_marker(readme: &str) -> Result<Option<Marker>, ManagedProjectError> {
    let Some(range) = marker_range(readme)? else {
        return Ok(None);
    };
    let payload_start = range.start + MARKER_BEGIN.len();
    let payload_end = range.end - MARKER_END.len();
    let payload = readme[payload_start..payload_end].trim_matches(['\r', '\n']);
    let lines = payload.lines().collect::<Vec<_>>();
    match lines.as_slice() {
        ["schema: 1", product_key, owner] => {
            let product_key = required_marker_value(product_key, "product-key: ", "product key")?;
            let owner = required_marker_value(owner, "owner: ", "owner")?;
            Ok(Some(Marker {
                identity: MarkerIdentity::Product {
                    product_key: product_key.to_owned(),
                },
                owner: owner.to_owned(),
                legacy: true,
            }))
        }
        ["schema: 2", "kind: product", product_key, owner] => {
            let product_key = required_marker_value(product_key, "product-key: ", "product key")?;
            let owner = required_marker_value(owner, "owner: ", "owner")?;
            Ok(Some(Marker {
                identity: MarkerIdentity::Product {
                    product_key: product_key.to_owned(),
                },
                owner: owner.to_owned(),
                legacy: false,
            }))
        }
        ["schema: 2", "kind: spec_portfolio", portfolio_id, source, owner, recovery_capsule] => {
            let portfolio_id =
                required_marker_value(portfolio_id, "portfolio-id: ", "portfolio id")?;
            let source = required_marker_value(source, "source: ", "source spec")?;
            let owner = required_marker_value(owner, "owner: ", "owner")?;
            let recovery_capsule =
                required_marker_value(recovery_capsule, "recovery-capsule: ", "recovery capsule")?;
            let recovery_capsule =
                PortfolioRecoveryCapsule::from_json(recovery_capsule.as_bytes())?;
            Ok(Some(Marker {
                identity: MarkerIdentity::SpecPortfolio {
                    portfolio_id: portfolio_id.to_owned(),
                    source: source.to_owned(),
                    recovery_capsule,
                },
                owner: owner.to_owned(),
                legacy: false,
            }))
        }
        _ => Err(ManagedProjectError::new(
            "GitHub Project managed marker has unsupported schema or shape",
        )),
    }
}

fn marker_range(readme: &str) -> Result<Option<Range<usize>>, ManagedProjectError> {
    let starts = readme.match_indices(MARKER_BEGIN).collect::<Vec<_>>();
    let ends = readme.match_indices(MARKER_END).collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    if starts.len() != 1 || ends.len() != 1 || starts[0].0 >= ends[0].0 {
        return Err(ManagedProjectError::new(
            "GitHub Project managed marker must contain exactly one complete block",
        ));
    }
    Ok(Some(starts[0].0..ends[0].0 + MARKER_END.len()))
}

fn required_marker_value<'a>(
    line: &'a str,
    prefix: &str,
    field: &str,
) -> Result<&'a str, ManagedProjectError> {
    line.strip_prefix(prefix)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ManagedProjectError::new(format!("GitHub Project managed marker has invalid {field}"))
        })
}

fn render_marker(
    identity: &ManagedProjectIdentity,
    owner: &str,
    recovery_capsule: Option<&PortfolioRecoveryCapsule>,
) -> Result<String, ManagedProjectError> {
    if owner.trim().is_empty() {
        return Err(ManagedProjectError::new(
            "managed GitHub Project marker owner must not be empty",
        ));
    }
    let payload = match identity {
        ManagedProjectIdentity::Product { product_key } => format!(
            "schema: 2\nkind: product\nproduct-key: {}\nowner: {owner}",
            product_key.as_str()
        ),
        ManagedProjectIdentity::SpecPortfolio(identity) => {
            let capsule = recovery_capsule.ok_or_else(|| {
                ManagedProjectError::new("spec portfolio marker requires a frozen recovery capsule")
            })?;
            if capsule.portfolio_id() != identity.portfolio_id().as_str() {
                return Err(ManagedProjectError::new(
                    "spec portfolio recovery capsule identity does not match marker identity",
                ));
            }
            let capsule = capsule.to_json()?;
            format!(
                "schema: 2\nkind: spec_portfolio\nportfolio-id: {}\nsource: {}\nowner: {owner}\nrecovery-capsule: {capsule}",
                identity.portfolio_id(),
                identity.source()
            )
        }
    };
    Ok(format!("{MARKER_BEGIN}\n{payload}\n{MARKER_END}"))
}

pub(super) fn recoverable_portfolio_capsule(
    readme: &str,
    identity: &ManagedProjectIdentity,
    owner: &str,
) -> Result<Option<PortfolioRecoveryCapsule>, ManagedProjectError> {
    let ManagedProjectIdentity::SpecPortfolio(expected) = identity else {
        return Ok(None);
    };
    let Some(marker) = parse_marker(readme)? else {
        return Ok(None);
    };
    let MarkerIdentity::SpecPortfolio {
        portfolio_id,
        source,
        recovery_capsule,
    } = marker.identity
    else {
        return Ok(None);
    };
    if portfolio_id != expected.portfolio_id().as_str() || source != expected.source().to_string() {
        return Ok(None);
    }
    if marker.owner != owner {
        return Err(ManagedProjectError::new(
            "spec portfolio marker owner conflicts with approved owner",
        ));
    }
    if recovery_capsule.portfolio_id() != expected.portfolio_id().as_str() {
        return Err(ManagedProjectError::new(
            "spec portfolio marker capsule identity conflicts with its marker",
        ));
    }
    Ok(Some(recovery_capsule))
}

fn require_lower_hex(value: &str, length: usize, field: &str) -> Result<(), ManagedProjectError> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ManagedProjectError::new(format!(
            "portfolio recovery capsule {field} must be lowercase hexadecimal"
        )));
    }
    Ok(())
}

fn validate_repository(repository: &str) -> Result<(), ManagedProjectError> {
    let parts = repository.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 100
                || !part.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
                || !part
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                || !part
                    .as_bytes()
                    .last()
                    .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
    {
        return Err(ManagedProjectError::new(
            "portfolio recovery capsule repository is not canonical",
        ));
    }
    Ok(())
}

fn json_error(error: serde_json::Error) -> ManagedProjectError {
    ManagedProjectError::new(format!("invalid GitHub Project response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use autospec_core::managed_project::{ProductKey, SourceSpecIdentity, SpecPortfolioIdentity};

    fn product() -> ManagedProjectIdentity {
        ManagedProjectIdentity::Product {
            product_key: ProductKey::new("autospec").unwrap(),
        }
    }

    fn portfolio() -> ManagedProjectIdentity {
        ManagedProjectIdentity::SpecPortfolio(SpecPortfolioIdentity::new(
            SourceSpecIdentity::new(
                "berlinguyinca/autospec",
                "docs/specs/automatic-projects.md",
                "0123456789abcdef0123456789abcdef01234567",
            )
            .unwrap(),
        ))
    }

    fn capsule_value() -> Value {
        serde_json::json!({
            "schema": "autospec.portfolio-recovery.v1",
            "portfolio_id": match portfolio() {
                ManagedProjectIdentity::SpecPortfolio(identity) => identity.portfolio_id().to_string(),
                ManagedProjectIdentity::Product { .. } => unreachable!(),
            },
            "plan_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "create_nonce": "00112233445566778899aabbccddeeff",
            "items": [{
                "item_key": "source-tracker",
                "repository": "berlinguyinca/autospec",
                "role": "source-tracker",
                "completion_policy": "closed-tracker",
                "local_parents": [],
                "dependencies": []
            }]
        })
    }

    fn capsule() -> PortfolioRecoveryCapsule {
        PortfolioRecoveryCapsule::from_value(&capsule_value()).unwrap()
    }

    #[test]
    fn marker_schema_migrates_legacy_product_without_losing_human_text() {
        let legacy = concat!(
            "# Human notes\n\n",
            "<!-- autospec-managed-project:begin -->\n",
            "schema: 1\n",
            "product-key: autospec\n",
            "owner: berlinguyinca\n",
            "<!-- autospec-managed-project:end -->\n",
            "\nKeep this too."
        );
        assert_eq!(
            classify_marker(legacy, &product(), "berlinguyinca", None).unwrap(),
            MarkerDisposition::Exact { legacy: true }
        );
        let migrated = upsert_marker(legacy, &product(), "berlinguyinca", None).unwrap();
        assert!(migrated.starts_with("# Human notes\n\n"));
        assert!(migrated.ends_with("\n\nKeep this too."));
        assert!(migrated.contains("schema: 2\nkind: product"));
        assert_eq!(
            classify_marker(&migrated, &product(), "berlinguyinca", None).unwrap(),
            MarkerDisposition::Exact { legacy: false }
        );
    }

    #[test]
    fn spec_portfolio_marker_requires_exact_identity_kind_and_capsule() {
        let capsule = capsule();
        let marker = upsert_marker("human", &portfolio(), "berlinguyinca", Some(&capsule)).unwrap();
        assert_eq!(
            classify_marker(&marker, &portfolio(), "berlinguyinca", Some(&capsule)).unwrap(),
            MarkerDisposition::Exact { legacy: false }
        );
        assert_eq!(
            classify_marker(&marker, &product(), "berlinguyinca", None).unwrap(),
            MarkerDisposition::Other
        );
        let mut wrong_capsule = capsule_value();
        wrong_capsule["plan_digest"] = Value::String(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        let wrong_capsule = PortfolioRecoveryCapsule::from_value(&wrong_capsule).unwrap();
        assert!(
            classify_marker(&marker, &portfolio(), "berlinguyinca", Some(&wrong_capsule))
                .unwrap_err()
                .to_string()
                .contains("recovery capsule conflicts")
        );
    }

    #[test]
    fn marker_parser_rejects_duplicate_and_mixed_schema_payloads() {
        let marker = upsert_marker("", &product(), "berlinguyinca", None).unwrap();
        assert!(classify_marker(
            &format!("{marker}\n{marker}"),
            &product(),
            "berlinguyinca",
            None
        )
        .is_err());
        let mixed = marker.replace("kind: product\n", "kind: product\nschema: 1\n");
        assert!(classify_marker(&mixed, &product(), "berlinguyinca", None).is_err());
    }

    #[test]
    fn project_candidate_pages_preserve_exact_titles_and_reject_truncation() {
        let candidates = parse_project_candidates(
            r#"{"projects":[{"number":7,"title":"Delivery [autospec:0011]"},{"number":8,"title":"Other"}],"totalCount":2}"#,
        )
        .unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].number, 7);
        assert_eq!(candidates[0].title, "Delivery [autospec:0011]");

        let truncated = r#"{"projects":[{"number":7,"title":"Delivery"}],"totalCount":2}"#;
        assert!(parse_project_candidates(truncated)
            .unwrap_err()
            .to_string()
            .contains("truncated"));
    }

    #[test]
    fn project_candidate_total_count_rejects_every_present_non_u64_shape() {
        for total_count in ["\"2\"", "-1", "{\"value\":2}", "18446744073709551616"] {
            let output = format!("{{\"projects\":[],\"totalCount\":{total_count}}}");
            assert!(parse_project_candidates(&output)
                .unwrap_err()
                .to_string()
                .contains("totalCount"));
        }
    }

    #[test]
    fn project_candidate_graphql_pages_are_exhaustive_and_duplicate_safe() {
        let pages = (0..6)
            .map(|page| {
                let start = page * 100 + 1;
                let nodes = (start..start + 100)
                    .map(|number| serde_json::json!({"number": number, "title": format!("P{number}")}))
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "data": {"repositoryOwner": {"projectsV2": {
                        "nodes": nodes,
                        "pageInfo": {
                            "hasNextPage": page < 5,
                            "endCursor": if page < 5 { Some(format!("cursor-{page}")) } else { None }
                        }
                    }}}
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            parse_project_candidates(&serde_json::to_string(&pages).unwrap())
                .unwrap()
                .len(),
            600
        );

        let duplicate = serde_json::json!([
            {"data":{"repositoryOwner":{"projectsV2":{"nodes":[{"number":7,"title":"one"}],"pageInfo":{"hasNextPage":true,"endCursor":"cursor"}}}}},
            {"data":{"repositoryOwner":{"projectsV2":{"nodes":[{"number":7,"title":"two"}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}
        ]);
        assert!(parse_project_candidates(&duplicate.to_string())
            .unwrap_err()
            .to_string()
            .contains("duplicate"));
    }
}
