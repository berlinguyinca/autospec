//! The on-disk artifact registry and audit log.
//!
//! Superseded artifacts stay immutable and queryable: a versioned artifact is
//! written once and never rewritten, so an old plan, graph, or Definition can
//! still be read after the Initiative has moved on.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AutospecError;

use super::ids::{InitiativeId, TaskId};

/// One artifact in an Initiative's registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitiativeArtifact {
    /// The Initiative record itself.
    Record,
    /// The source Specification.
    Spec,
    /// A Definition version.
    Definition {
        /// The Definition version.
        version: u32,
    },
    /// The discovered repository manifest.
    WorkspaceRepositories,
    /// The discovered per-repository capability records.
    WorkspacePermissions,
    /// An architecture plan version.
    ArchitecturePlan {
        /// The plan version.
        version: u32,
    },
    /// A task graph version.
    TaskGraph {
        /// The graph version.
        version: u32,
    },
    /// A task-local plan version.
    TaskPlan {
        /// The task.
        task: TaskId,
        /// The task plan version.
        version: u32,
    },
    /// The result of one implementation attempt.
    ImplementationResult {
        /// The task.
        task: TaskId,
        /// The attempt sequence.
        attempt: u32,
    },
    /// A task's test report.
    TestReport {
        /// The task.
        task: TaskId,
    },
    /// A task's review.
    Review {
        /// The task.
        task: TaskId,
    },
    /// The cross-repository integration report.
    IntegrationReport,
    /// Evidence records collected across the Initiative.
    Evidence,
    /// Approved waivers for requirements that will not be verified.
    Waivers,
    /// The requirement coverage matrix.
    RequirementsMatrix,
    /// The final verification report.
    FinalReport,
    /// The rendered GitHub projection.
    GithubProjection,
}

impl InitiativeArtifact {
    /// The path relative to the Initiative root.
    pub fn relative_path(&self) -> PathBuf {
        match self {
            InitiativeArtifact::Record => PathBuf::from("initiative.json"),
            InitiativeArtifact::Spec => PathBuf::from("spec/spec.md"),
            InitiativeArtifact::Definition { version } => {
                PathBuf::from(format!("definition/definition-v{version}.json"))
            }
            InitiativeArtifact::WorkspaceRepositories => {
                PathBuf::from("workspace/repositories.json")
            }
            InitiativeArtifact::WorkspacePermissions => PathBuf::from("workspace/permissions.json"),
            InitiativeArtifact::ArchitecturePlan { version } => {
                PathBuf::from(format!("plans/architecture-plan-v{version}.json"))
            }
            InitiativeArtifact::TaskGraph { version } => {
                PathBuf::from(format!("graph/task-graph-v{version}.json"))
            }
            InitiativeArtifact::TaskPlan { task, version } => PathBuf::from(format!(
                "tasks/{}/task-plan-v{version}.json",
                task.as_str()
            )),
            InitiativeArtifact::ImplementationResult { task, attempt } => PathBuf::from(format!(
                "tasks/{}/implementation-result-a{attempt}.json",
                task.as_str()
            )),
            InitiativeArtifact::TestReport { task } => {
                PathBuf::from(format!("tasks/{}/test-report.md", task.as_str()))
            }
            InitiativeArtifact::Review { task } => {
                PathBuf::from(format!("tasks/{}/review.md", task.as_str()))
            }
            InitiativeArtifact::IntegrationReport => {
                PathBuf::from("integration/integration-report.md")
            }
            InitiativeArtifact::Evidence => PathBuf::from("verification/evidence.json"),
            InitiativeArtifact::Waivers => PathBuf::from("verification/waivers.json"),
            InitiativeArtifact::RequirementsMatrix => {
                PathBuf::from("verification/requirements-matrix.json")
            }
            InitiativeArtifact::FinalReport => PathBuf::from("verification/final-report.md"),
            InitiativeArtifact::GithubProjection => PathBuf::from("projections/github.json"),
        }
    }

    /// Whether the artifact may never be rewritten once it exists.
    ///
    /// Versioned artifacts and per-attempt results are evidence; rewriting one
    /// would erase the history a replan or an audit depends on.
    pub fn is_immutable(&self) -> bool {
        matches!(
            self,
            InitiativeArtifact::Definition { .. }
                | InitiativeArtifact::ArchitecturePlan { .. }
                | InitiativeArtifact::TaskGraph { .. }
                | InitiativeArtifact::TaskPlan { .. }
                | InitiativeArtifact::ImplementationResult { .. }
        )
    }
}

/// One entry in the Initiative's append-only audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Unix seconds.
    pub at: u64,
    /// The event kind, e.g. `task.leased` or `plan.superseded`.
    pub kind: String,
    /// The Initiative the event belongs to.
    pub initiative: InitiativeId,
    /// The task, when the event is task-scoped.
    #[serde(default)]
    pub task: Option<TaskId>,
    /// Who or what caused the event.
    pub actor: String,
    /// Event-specific fields.
    #[serde(default)]
    pub detail: BTreeMap<String, String>,
}

impl AuditEvent {
    /// An Initiative-scoped event.
    pub fn new(
        at: u64,
        kind: impl Into<String>,
        initiative: InitiativeId,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            at,
            kind: kind.into(),
            initiative,
            task: None,
            actor: actor.into(),
            detail: BTreeMap::new(),
        }
    }

    /// The same event, scoped to a task.
    pub fn for_task(mut self, task: TaskId) -> Self {
        self.task = Some(task);
        self
    }

    /// The same event with one more detail field.
    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.detail.insert(key.into(), value.into());
        self
    }
}

/// The artifact registry for one Initiative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitiativeStore {
    initiative: InitiativeId,
    root: PathBuf,
}

impl InitiativeStore {
    /// The registry rooted at `<root>/.autospec/initiatives/<initiative>`.
    pub fn new(root: &Path, initiative: InitiativeId) -> Self {
        let root = root
            .join(".autospec")
            .join("initiatives")
            .join(initiative.as_str());
        Self { initiative, root }
    }

    /// The Initiative this registry belongs to.
    pub fn initiative(&self) -> &InitiativeId {
        &self.initiative
    }

    /// The registry root on disk.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The absolute path of `artifact`.
    pub fn path(&self, artifact: &InitiativeArtifact) -> PathBuf {
        self.root.join(artifact.relative_path())
    }

    /// Create the directory layout.
    pub fn initialize(&self) -> Result<(), AutospecError> {
        for directory in [
            "spec",
            "definition",
            "workspace",
            "plans",
            "graph",
            "tasks",
            "integration",
            "verification",
            "projections",
            "audit",
        ] {
            let path = self.root.join(directory);
            fs::create_dir_all(&path)
                .map_err(|error| AutospecError::io("create", path.display().to_string(), error))?;
        }
        Ok(())
    }

    /// Whether `artifact` exists.
    pub fn exists(&self, artifact: &InitiativeArtifact) -> bool {
        self.path(artifact).is_file()
    }

    /// Write `contents` to `artifact`, refusing to rewrite immutable artifacts.
    pub fn write(
        &self,
        artifact: &InitiativeArtifact,
        contents: &str,
    ) -> Result<PathBuf, AutospecError> {
        let path = self.path(artifact);
        if artifact.is_immutable() && path.exists() {
            return Err(AutospecError::invariant(format!(
                "{} is immutable and already exists",
                artifact.relative_path().display()
            )));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AutospecError::io("create", parent.display().to_string(), error)
            })?;
        }
        fs::write(&path, contents)
            .map_err(|error| AutospecError::io("write", path.display().to_string(), error))?;
        Ok(path)
    }

    /// Serialize `value` into `artifact` as pretty JSON.
    pub fn write_json<T: Serialize>(
        &self,
        artifact: &InitiativeArtifact,
        value: &T,
    ) -> Result<PathBuf, AutospecError> {
        let rendered = serde_json::to_string_pretty(value).map_err(|error| {
            AutospecError::parse(artifact.relative_path().display().to_string(), error.to_string())
        })?;
        self.write(artifact, &format!("{rendered}\n"))
    }

    /// Read `artifact` as text.
    pub fn read(&self, artifact: &InitiativeArtifact) -> Result<String, AutospecError> {
        let path = self.path(artifact);
        fs::read_to_string(&path)
            .map_err(|error| AutospecError::io("read", path.display().to_string(), error))
    }

    /// Deserialize `artifact` as JSON.
    pub fn read_json<T: for<'de> Deserialize<'de>>(
        &self,
        artifact: &InitiativeArtifact,
    ) -> Result<T, AutospecError> {
        let contents = self.read(artifact)?;
        serde_json::from_str(&contents).map_err(|error| {
            AutospecError::parse(artifact.relative_path().display().to_string(), error.to_string())
        })
    }

    /// Every version present for a versioned artifact family, ascending.
    pub fn versions(&self, family: ArtifactFamily) -> Vec<u32> {
        let directory = self.root.join(family.directory());
        let Ok(entries) = fs::read_dir(&directory) else {
            return Vec::new();
        };
        let mut versions = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                let stem = name.strip_prefix(family.prefix())?;
                let stem = stem.strip_suffix(".json")?;
                stem.strip_prefix('v')?.parse::<u32>().ok()
            })
            .collect::<Vec<_>>();
        versions.sort_unstable();
        versions
    }

    /// The highest version present for a versioned artifact family.
    pub fn latest_version(&self, family: ArtifactFamily) -> Option<u32> {
        self.versions(family).into_iter().next_back()
    }

    /// Append one event to the audit log.
    pub fn append_event(&self, event: &AuditEvent) -> Result<(), AutospecError> {
        use std::io::Write;

        let path = self.root.join("audit/events.jsonl");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AutospecError::io("create", parent.display().to_string(), error)
            })?;
        }
        let rendered = serde_json::to_string(event)
            .map_err(|error| AutospecError::parse("audit event", error.to_string()))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| AutospecError::io("open", path.display().to_string(), error))?;
        writeln!(file, "{rendered}")
            .map_err(|error| AutospecError::io("append to", path.display().to_string(), error))
    }

    /// Read the audit log in order.
    pub fn events(&self) -> Result<Vec<AuditEvent>, AutospecError> {
        let path = self.root.join("audit/events.jsonl");
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(AutospecError::io("read", path.display().to_string(), error))
            }
        };
        contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|error| AutospecError::parse("audit event", error.to_string()))
            })
            .collect()
    }
}

/// A family of versioned artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactFamily {
    /// `definition/definition-v*.json`
    Definition,
    /// `plans/architecture-plan-v*.json`
    ArchitecturePlan,
    /// `graph/task-graph-v*.json`
    TaskGraph,
}

impl ArtifactFamily {
    /// The directory the family lives in.
    pub fn directory(&self) -> &'static str {
        match self {
            ArtifactFamily::Definition => "definition",
            ArtifactFamily::ArchitecturePlan => "plans",
            ArtifactFamily::TaskGraph => "graph",
        }
    }

    /// The file name prefix before the version.
    pub fn prefix(&self) -> &'static str {
        match self {
            ArtifactFamily::Definition => "definition-",
            ArtifactFamily::ArchitecturePlan => "architecture-plan-",
            ArtifactFamily::TaskGraph => "task-graph-",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn initiative() -> InitiativeId {
        InitiativeId::parse("INIT-2026-0042").expect("valid initiative id")
    }

    fn temp_store(name: &str) -> InitiativeStore {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("autospec-initiative-{name}-{nonce}"));
        fs::create_dir_all(&root).expect("temp root");
        let store = InitiativeStore::new(&root, initiative());
        store.initialize().expect("layout");
        store
    }

    #[test]
    fn the_layout_follows_the_specification() {
        let store = temp_store("layout");

        assert!(store.root().ends_with(".autospec/initiatives/INIT-2026-0042"));
        assert_eq!(
            InitiativeArtifact::TaskPlan {
                task: TaskId::parse("TASK-0017").expect("valid task id"),
                version: 1,
            }
            .relative_path(),
            PathBuf::from("tasks/TASK-0017/task-plan-v1.json")
        );
        assert!(store.root().join("verification").is_dir());
        assert!(store.root().join("audit").is_dir());
    }

    #[test]
    fn a_superseded_plan_version_stays_readable_after_a_newer_one_lands() {
        let store = temp_store("versions");

        store
            .write(&InitiativeArtifact::ArchitecturePlan { version: 1 }, "v1")
            .expect("v1 written");
        store
            .write(&InitiativeArtifact::ArchitecturePlan { version: 2 }, "v2")
            .expect("v2 written");

        assert_eq!(store.versions(ArtifactFamily::ArchitecturePlan), vec![1, 2]);
        assert_eq!(
            store
                .read(&InitiativeArtifact::ArchitecturePlan { version: 1 })
                .expect("v1 readable"),
            "v1"
        );
    }

    #[test]
    fn json_versions_are_discoverable_and_ordered() {
        let store = temp_store("json-versions");

        for version in [2, 1, 3] {
            store
                .write_json(
                    &InitiativeArtifact::TaskGraph { version },
                    &BTreeMap::from([("version".to_string(), version)]),
                )
                .expect("written");
        }

        assert_eq!(store.versions(ArtifactFamily::TaskGraph), vec![1, 2, 3]);
        assert_eq!(store.latest_version(ArtifactFamily::TaskGraph), Some(3));
    }

    #[test]
    fn an_immutable_artifact_may_not_be_rewritten() {
        let store = temp_store("immutable");
        store
            .write(&InitiativeArtifact::Definition { version: 1 }, "first")
            .expect("first write");

        let error = store
            .write(&InitiativeArtifact::Definition { version: 1 }, "second")
            .expect_err("rewrite refused");

        assert!(error.to_string().contains("immutable"), "{error}");
        assert_eq!(
            store
                .read(&InitiativeArtifact::Definition { version: 1 })
                .expect("still readable"),
            "first"
        );
    }

    #[test]
    fn a_derived_projection_may_be_rewritten() {
        let store = temp_store("projection");
        store
            .write(&InitiativeArtifact::GithubProjection, "{}")
            .expect("first write");

        store
            .write(&InitiativeArtifact::GithubProjection, "{\"rebuilt\":true}")
            .expect("a projection is rebuilt, not preserved");

        assert!(store
            .read(&InitiativeArtifact::GithubProjection)
            .expect("readable")
            .contains("rebuilt"));
    }

    #[test]
    fn audit_events_append_in_order_and_round_trip() {
        let store = temp_store("audit");
        let task = TaskId::parse("TASK-0017").expect("valid task id");

        store
            .append_event(&AuditEvent::new(
                1_772_000_000,
                "initiative.created",
                initiative(),
                "orchestrator",
            ))
            .expect("first event");
        store
            .append_event(
                &AuditEvent::new(1_772_000_060, "task.leased", initiative(), "scheduler")
                    .for_task(task.clone())
                    .with_detail("session", "aspec-INIT-0042-TASK-0017-impl-a1"),
            )
            .expect("second event");

        let events = store.events().expect("readable");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "initiative.created");
        assert_eq!(events[1].task, Some(task));
        assert_eq!(
            events[1].detail["session"],
            "aspec-INIT-0042-TASK-0017-impl-a1"
        );
    }

    #[test]
    fn an_empty_audit_log_reads_as_no_events() {
        let store = temp_store("empty-audit");

        assert!(store.events().expect("readable").is_empty());
    }
}
