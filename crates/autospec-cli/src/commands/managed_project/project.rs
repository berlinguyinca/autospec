use super::{store, ManagedProjectError, ManagedProjectStore, ProductLock};
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
            format!("project:create-identity:{}", self.product_key.as_str()),
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
        if self.binding.project_node_id.is_some()
            && project_binding_payload(&self.binding) != Some(payload.clone())
        {
            return Err(ManagedProjectError::new(
                "managed project binding conflicts with the verified remote project",
            ));
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
            format!("project:bind:{}", self.product_key.as_str()),
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
