use super::{store, ManagedProjectError, ManagedProjectStore, ProductLock};
use autospec_core::autonomous::waterfall::sha256_hex;
use autospec_core::managed_project::ManagedProjectBinding;
use serde_json::{json, Value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProject {
    pub node_id: String,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub owner: String,
    pub readme: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectIdentity {
    pub(super) owner: String,
    pub(super) node_id: String,
    pub(super) number: u64,
    pub(super) url: String,
    pub(super) title: String,
}

impl ManagedProjectStore {
    pub(super) fn record_created_project(
        &mut self,
        project: &RemoteProject,
    ) -> Result<(), ManagedProjectError> {
        let identity = ProjectIdentity::from_remote(project)?;
        let payload = project_identity_payload(&identity);
        let _lock = ProductLock::acquire(&self.root.join(store::LOCK_FILE))?;
        self.refresh_from_journal()?;
        if self.binding.project_node_id.is_some() {
            return Err(ManagedProjectError::new(
                "cannot record provisional identity after final project binding",
            ));
        }
        if self
            .provisional_project()
            .is_some_and(|existing| !existing.same_immutable_identity(&identity))
        {
            return Err(ManagedProjectError::new(
                "provisional project identity conflicts with the created project",
            ));
        }
        self.append_event_locked(
            format!("project:create-identity:{}", self.event_identity_key()),
            "project-created",
            payload,
        )
    }

    pub fn record_project(
        &mut self,
        owner: &str,
        node_id: &str,
        number: u64,
        url: &str,
        title: &str,
    ) -> Result<(), ManagedProjectError> {
        if [owner, node_id, url, title]
            .iter()
            .any(|value| value.trim().is_empty())
            || number == 0
        {
            return Err(ManagedProjectError::new(
                "managed project identity fields must not be empty",
            ));
        }
        let _lock = ProductLock::acquire(&self.root.join(store::LOCK_FILE))?;
        self.refresh_from_journal()?;
        let identity = ProjectIdentity {
            owner: owner.to_owned(),
            node_id: node_id.to_owned(),
            number,
            url: url.to_owned(),
            title: title.to_owned(),
        };
        let payload = project_identity_payload(&identity);
        if let Some(existing) = project_binding_payload(&self.binding) {
            let existing = parse_project_identity(&existing)?;
            if !existing.same_immutable_identity(&identity) {
                return Err(ManagedProjectError::new(
                    "managed project binding conflicts with the verified remote project",
                ));
            }
        }
        if self
            .provisional_project()
            .is_some_and(|provisional| !provisional.same_immutable_identity(&identity))
        {
            return Err(ManagedProjectError::new(
                "verified project conflicts with provisional created identity",
            ));
        }
        self.append_event_locked(
            format!(
                "project:bind:{}:{}",
                self.event_identity_key(),
                sha256_hex(payload.to_string().as_bytes())
            ),
            "project-bound",
            payload,
        )
    }
}

impl ProjectIdentity {
    fn from_remote(project: &RemoteProject) -> Result<Self, ManagedProjectError> {
        let identity = Self {
            owner: project.owner.clone(),
            node_id: project.node_id.clone(),
            number: project.number,
            url: project.url.clone(),
            title: project.title.clone(),
        };
        validate_project_identity(&identity)?;
        Ok(identity)
    }

    pub(super) fn same_immutable_identity(&self, other: &Self) -> bool {
        self.owner == other.owner && self.node_id == other.node_id && self.number == other.number
    }
}

fn validate_project_identity(identity: &ProjectIdentity) -> Result<(), ManagedProjectError> {
    if [
        &identity.owner,
        &identity.node_id,
        &identity.url,
        &identity.title,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
        || identity.number == 0
    {
        return Err(ManagedProjectError::new(
            "managed project identity fields must not be empty",
        ));
    }
    Ok(())
}

fn project_identity_payload(identity: &ProjectIdentity) -> Value {
    json!({
        "owner": identity.owner,
        "node_id": identity.node_id,
        "number": identity.number,
        "url": identity.url,
        "title": identity.title,
    })
}

pub(crate) fn parse_project_identity(
    payload: &Value,
) -> Result<ProjectIdentity, ManagedProjectError> {
    let object = payload
        .as_object()
        .ok_or_else(|| ManagedProjectError::new("project identity payload must be an object"))?;
    let string = |field: &str| {
        object
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                ManagedProjectError::new(format!("project identity payload has invalid {field}"))
            })
    };
    let identity = ProjectIdentity {
        owner: string("owner")?,
        node_id: string("node_id")?,
        number: object
            .get("number")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                ManagedProjectError::new("project identity payload has invalid number")
            })?,
        url: string("url")?,
        title: string("title")?,
    };
    validate_project_identity(&identity)?;
    Ok(identity)
}

pub(crate) fn apply_project_binding(
    binding: &mut ManagedProjectBinding,
    payload: &Value,
) -> Result<(), ManagedProjectError> {
    let identity = parse_project_identity(payload)?;
    binding.owner = Some(identity.owner);
    binding.project_node_id = Some(identity.node_id);
    binding.project_number = Some(identity.number);
    binding.project_url = Some(identity.url);
    binding.project_title = Some(identity.title);
    Ok(())
}

pub(crate) fn project_binding_payload(binding: &ManagedProjectBinding) -> Option<Value> {
    Some(json!({
        "owner": binding.owner.as_deref()?,
        "node_id": binding.project_node_id.as_deref()?,
        "number": binding.project_number?,
        "url": binding.project_url.as_deref()?,
        "title": binding.project_title.as_deref()?,
    }))
}

// ── Managed delivery field shape (spec "Project shape") ──────────────────────

pub const DELIVERY_FIELD_NAME: &str = "Autospec delivery";
pub const REPOSITORY_FIELD_NAME: &str = "Repository";

/// The exact `Autospec delivery` single-select options, in canonical order.
pub const DELIVERY_OPTIONS: [&str; 10] = [
    "Planned",
    "Ready",
    "Running",
    "PR Open",
    "Review",
    "Verifying",
    "Blocked",
    "Failed",
    "Unknown",
    "Done",
];
pub const WORK_KIND_OPTIONS: [&str; 4] = ["Umbrella", "Implementation", "Audit", "Prerequisite"];
pub const CI_OPTIONS: [&str; 4] = ["Not started", "Pending", "Passing", "Failing"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedFieldKind {
    SingleSelect,
    Text,
    Date,
    /// GitHub built-in system field: verified by data type, never created.
    Repository,
}

impl ManagedFieldKind {
    /// The GitHub Projects v2 data type this kind maps to.
    pub fn data_type(self) -> &'static str {
        match self {
            Self::SingleSelect => "SINGLE_SELECT",
            Self::Text => "TEXT",
            Self::Date => "DATE",
            Self::Repository => "REPOSITORY",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedFieldSpec {
    pub name: &'static str,
    pub kind: ManagedFieldKind,
    /// Managed single-select options in canonical order; empty for other kinds.
    pub options: &'static [&'static str],
    /// Built-in system fields are verified and never treated as Autospec-owned.
    pub built_in: bool,
}

impl ManagedFieldSpec {
    /// Stable portfolio operation ID that journals ownership of this custom field.
    pub fn operation_id(&self) -> String {
        format!("field:{}", self.name.to_ascii_lowercase().replace(' ', "-"))
    }
}

/// The complete managed field set required by the spec, in spec order.
pub fn required_managed_fields() -> Vec<ManagedFieldSpec> {
    vec![
        ManagedFieldSpec {
            name: DELIVERY_FIELD_NAME,
            kind: ManagedFieldKind::SingleSelect,
            options: &DELIVERY_OPTIONS,
            built_in: false,
        },
        ManagedFieldSpec {
            name: REPOSITORY_FIELD_NAME,
            kind: ManagedFieldKind::Repository,
            options: &[],
            built_in: true,
        },
        ManagedFieldSpec {
            name: "Work kind",
            kind: ManagedFieldKind::SingleSelect,
            options: &WORK_KIND_OPTIONS,
            built_in: false,
        },
        ManagedFieldSpec {
            name: "Source spec",
            kind: ManagedFieldKind::Text,
            options: &[],
            built_in: false,
        },
        ManagedFieldSpec {
            name: "Depends on",
            kind: ManagedFieldKind::Text,
            options: &[],
            built_in: false,
        },
        ManagedFieldSpec {
            name: "Pull request",
            kind: ManagedFieldKind::Text,
            options: &[],
            built_in: false,
        },
        ManagedFieldSpec {
            name: "CI",
            kind: ManagedFieldKind::SingleSelect,
            options: &CI_OPTIONS,
            built_in: false,
        },
        ManagedFieldSpec {
            name: "Last activity",
            kind: ManagedFieldKind::Date,
            options: &[],
            built_in: false,
        },
    ]
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteFieldOption {
    pub node_id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteField {
    pub node_id: String,
    pub name: String,
    /// GitHub data type as reported by the API (`SINGLE_SELECT`, `TEXT`, `DATE`,
    /// `REPOSITORY`, …).
    pub data_type: String,
    pub options: Vec<RemoteFieldOption>,
}

/// Parses `gh project field-list --format json`: `{"fields": [{id, name, dataType,
/// options}, …]}`. Unknown keys are ignored so GitHub can extend the shape; a
/// malformed entry is a definitive parse failure, never a guessed field.
fn field_option(option: &Value) -> Result<RemoteFieldOption, ManagedProjectError> {
    let string = |name: &str| {
        option
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                ManagedProjectError::new(format!("GitHub field option has invalid {name}"))
            })
    };
    Ok(RemoteFieldOption {
        node_id: string("id")?,
        name: string("name")?,
    })
}

fn field_options(field: &Value) -> Result<Vec<RemoteFieldOption>, ManagedProjectError> {
    match field.get("options") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(options) => options
            .as_array()
            .ok_or_else(|| {
                ManagedProjectError::new("GitHub field options must be an array or null")
            })?
            .iter()
            .map(field_option)
            .collect(),
    }
}

fn field_data_type(field: &Value, context: &str) -> Result<String, ManagedProjectError> {
    field
        .get("dataType")
        .or_else(|| field.get("data_type"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ManagedProjectError::new(format!("{context} has invalid data type")))
}

fn field_string(field: &Value, name: &str, context: &str) -> Result<String, ManagedProjectError> {
    field
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ManagedProjectError::new(format!("{context} has invalid {name}")))
}

pub fn parse_remote_fields(output: &str) -> Result<Vec<RemoteField>, ManagedProjectError> {
    let value: Value = serde_json::from_str(output)
        .map_err(|error| ManagedProjectError::new(format!("invalid field list: {error}")))?;
    let fields = value
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| ManagedProjectError::new("GitHub field list has no fields array"))?;
    fields
        .iter()
        .map(|field| {
            Ok(RemoteField {
                node_id: field_string(field, "id", "GitHub field")?,
                name: field_string(field, "name", "GitHub field")?,
                data_type: field_data_type(field, "GitHub field")?,
                options: field_options(field)?,
            })
        })
        .collect()
}

/// Parses `gh project field-create --format json`: either `{"field": {…}}` or the
/// bare field object `{id, name, dataType, options}`.
pub fn parse_created_field(output: &str) -> Result<RemoteField, ManagedProjectError> {
    let value: Value = serde_json::from_str(output)
        .map_err(|error| ManagedProjectError::new(format!("invalid created field: {error}")))?;
    let field = value.get("field").unwrap_or(&value);
    Ok(RemoteField {
        node_id: field_string(field, "id", "created field")?,
        name: field_string(field, "name", "created field")?,
        data_type: field_data_type(field, "created field")?,
        options: field_options(field)?,
    })
}

/// Journal state of an Autospec field-ownership operation, derived from the
/// durable `field:<slug>` portfolio operation (intent → sent → acknowledged).
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum FieldOwnership {
    /// Intent journaled; nothing dispatched yet.
    Intent,
    /// Create dispatched; the response is not acknowledged.
    Sent,
    /// Create acknowledged; Autospec owns the field.
    Acknowledged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldPlanEntry {
    /// Built-in field verified by name and system data type. Never created.
    VerifiedBuiltIn { name: &'static str, node_id: String },
    /// Autospec-owned custom field verified type- and option-compatible; option
    /// node IDs are re-read from the remote field so they stay the source of truth.
    VerifiedOwned {
        name: &'static str,
        node_id: String,
        options: Vec<RemoteFieldOption>,
    },
    /// Custom field absent from the remote Project; must be created exactly as
    /// specified.
    Create { spec: ManagedFieldSpec },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldPlan {
    pub entries: Vec<FieldPlanEntry>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum FieldResolutionError {
    /// Two or more remote fields share a name; name-addressed operations are
    /// ambiguous and must be resolved by a human.
    AmbiguousField { name: String, matches: usize },
    /// An existing field with a managed name carries a different data type.
    IncompatibleFieldType {
        name: String,
        expected: String,
        actual: String,
    },
    /// An existing field with a managed name was not created by Autospec. It is
    /// never deleted, renamed, or repurposed.
    HumanOwnedField { name: String },
    /// An Autospec-owned field disappeared from the remote Project.
    OwnedFieldMissing { name: String },
    /// A managed single-select option is absent from an existing field; GitHub
    /// exposes no option-add mutation, so this blocks.
    MissingManagedOption { field: String, option: String },
    /// The built-in Repository field is absent; it is verified, never created.
    MissingBuiltInRepository,
    /// A create response does not match the managed spec it was dispatched with.
    CreatedFieldMismatch {
        field: String,
        expected: String,
        actual: String,
    },
}

impl std::fmt::Display for FieldResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AmbiguousField { name, matches } => write!(
                formatter,
                "GitHub Project has {matches} fields named {name}; refusing to guess"
            ),
            Self::IncompatibleFieldType { name, expected, actual } => write!(
                formatter,
                "field {name} has data type {actual}, expected {expected}; refusing to repurpose it"
            ),
            Self::HumanOwnedField { name } => write!(
                formatter,
                "field {name} is not Autospec-owned; refusing to delete, rename, or repurpose it"
            ),
            Self::OwnedFieldMissing { name } => write!(
                formatter,
                "Autospec-owned field {name} is missing from the GitHub Project"
            ),
            Self::MissingManagedOption { field, option } => write!(
                formatter,
                "field {field} is missing managed option {option}; option sets are fixed at creation"
            ),
            Self::MissingBuiltInRepository => formatter
                .write_str("built-in Repository field is missing; it is verified, never created"),
            Self::CreatedFieldMismatch { field, expected, actual } => write!(
                formatter,
                "created field {field} reports {actual}, expected {expected}"
            ),
        }
    }
}

/// Resolves the required managed field set against the remote fields and the
/// journaled ownership operations. Pure: it plans, it never mutates.
pub fn resolve_managed_fields(
    remote: &[RemoteField],
    ownership: &[(String, FieldOwnership)],
) -> Result<FieldPlan, FieldResolutionError> {
    // A duplicate name anywhere breaks name-addressed field operations, so it is
    // checked before the managed set is resolved at all.
    let mut duplicate = None;
    let mut names: Vec<&str> = remote.iter().map(|field| field.name.as_str()).collect();
    names.sort_unstable();
    for window in names.windows(2) {
        if window[0] == window[1] && duplicate.is_none() {
            let count = names.iter().filter(|name| **name == window[0]).count();
            duplicate = Some(FieldResolutionError::AmbiguousField {
                name: window[0].to_owned(),
                matches: count,
            });
        }
    }
    if let Some(error) = duplicate {
        return Err(error);
    }
    let mut entries = Vec::with_capacity(required_managed_fields().len());
    for spec in required_managed_fields() {
        let field = remote.iter().find(|field| field.name == spec.name).cloned();
        let ownership_state = ownership
            .iter()
            .find(|(operation_id, _)| operation_id == &spec.operation_id())
            .map(|(_, state)| *state);
        match field {
            None => {
                if spec.built_in {
                    return Err(FieldResolutionError::MissingBuiltInRepository);
                }
                if matches!(ownership_state, Some(FieldOwnership::Acknowledged)) {
                    return Err(FieldResolutionError::OwnedFieldMissing {
                        name: spec.name.to_owned(),
                    });
                }
                entries.push(FieldPlanEntry::Create { spec });
            }
            Some(field) => {
                if spec.built_in {
                    if field.data_type != ManagedFieldKind::Repository.data_type() {
                        return Err(FieldResolutionError::IncompatibleFieldType {
                            name: spec.name.to_owned(),
                            expected: ManagedFieldKind::Repository.data_type().to_owned(),
                            actual: field.data_type,
                        });
                    }
                    entries.push(FieldPlanEntry::VerifiedBuiltIn {
                        name: spec.name,
                        node_id: field.node_id,
                    });
                    continue;
                }
                if field.data_type != spec.kind.data_type() {
                    return Err(FieldResolutionError::IncompatibleFieldType {
                        name: spec.name.to_owned(),
                        expected: spec.kind.data_type().to_owned(),
                        actual: field.data_type,
                    });
                }
                // Ownership requires a dispatched or acknowledged create: an intent
                // alone never claims a field a human may have created in the meantime.
                if !matches!(
                    ownership_state,
                    Some(FieldOwnership::Sent) | Some(FieldOwnership::Acknowledged)
                ) {
                    return Err(FieldResolutionError::HumanOwnedField {
                        name: spec.name.to_owned(),
                    });
                }
                let mut options = Vec::with_capacity(spec.options.len());
                for option in spec.options {
                    let remote_option = field
                        .options
                        .iter()
                        .find(|remote_option| &remote_option.name == option)
                        .ok_or_else(|| FieldResolutionError::MissingManagedOption {
                            field: spec.name.to_owned(),
                            option: (*option).to_owned(),
                        })?;
                    options.push(remote_option.clone());
                }
                entries.push(FieldPlanEntry::VerifiedOwned {
                    name: spec.name,
                    node_id: field.node_id,
                    options,
                });
            }
        }
    }
    Ok(FieldPlan { entries })
}

/// Verifies a create response against the spec it was dispatched with: the name,
/// data type, and the exact managed option set must all match.
pub fn validate_created_field(
    created: &RemoteField,
    spec: &ManagedFieldSpec,
) -> Result<(), FieldResolutionError> {
    if created.name != spec.name {
        return Err(FieldResolutionError::CreatedFieldMismatch {
            field: created.name.clone(),
            expected: spec.name.to_owned(),
            actual: created.name.clone(),
        });
    }
    if created.data_type != spec.kind.data_type() {
        return Err(FieldResolutionError::CreatedFieldMismatch {
            field: created.name.clone(),
            expected: spec.kind.data_type().to_owned(),
            actual: created.data_type.clone(),
        });
    }
    let reported: Vec<&str> = created
        .options
        .iter()
        .map(|option| option.name.as_str())
        .collect();
    let expected: Vec<&str> = spec.options.to_vec();
    if reported.len() != expected.len() || reported.iter().any(|name| !expected.contains(name)) {
        return Err(FieldResolutionError::CreatedFieldMismatch {
            field: created.name.clone(),
            expected: expected.join(", "),
            actual: reported.join(", "),
        });
    }
    Ok(())
}

/// Whether the GitHub surface exposes Project v2 view mutation.
pub enum ViewCapability {
    /// View mutation is supported: a table grouped by `Autospec delivery` and a
    /// board of its columns can be created.
    Supported,
    /// No supported view mutation (the gh CLI `project` surface has no view
    /// create, and the shared transport carries no view command).
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewSetup {
    /// Views were provisioned by Autospec.
    Provisioned { table: String, board: String },
    /// View setup is manual: the exact Project URL and the field list a human
    /// groups by. Never claims the default views exist.
    Manual {
        project_url: String,
        fields: Vec<String>,
    },
}

/// Plans the view setup for a provisioned Project. With unsupported view
/// mutation the managed README summary and fields still answer completed,
/// active, blocked, failed, and outstanding counts; the views are convenience.
pub fn plan_view_setup(capability: ViewCapability, project_url: &str) -> ViewSetup {
    match capability {
        ViewCapability::Supported => ViewSetup::Provisioned {
            table: format!("{project_url} — table grouped by {DELIVERY_FIELD_NAME}"),
            board: format!("{project_url} — board of {DELIVERY_FIELD_NAME} columns"),
        },
        ViewCapability::Unsupported => ViewSetup::Manual {
            project_url: project_url.to_owned(),
            fields: required_managed_fields()
                .into_iter()
                .map(|spec| spec.name.to_owned())
                .collect(),
        },
    }
}

/// The outcome of one portfolio Project-shape provisioning pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioShapeReport {
    /// The resolved field plan after provisioning (verified and created entries).
    pub fields: Vec<FieldPlanEntry>,
    pub view_setup: ViewSetup,
    /// Items already present in the Project at check time.
    pub items_present: usize,
    /// Items added during this pass.
    pub items_added: usize,
}
