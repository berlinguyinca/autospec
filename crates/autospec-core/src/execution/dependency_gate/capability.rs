//! Rule 1 — capability-aware dispatch routing.
//!
//! A criterion that names a dependency the executor cannot provide cannot be
//! verified by that executor, no matter what its tests report. The old routing
//! asked only "is an executor free?" and dispatched the task anyway, which
//! pushed the agent into inventing the dependency. The routing decision is
//! therefore made against the *requirements*, and when no executor satisfies
//! them the answer is a hold that stays in the queue:
//! [`CapabilityUnavailable`], wire code `CAPABILITY-UNAVAILABLE`.

use std::collections::BTreeSet;

use super::token_present;

/// A dependency an acceptance criterion can name and an executor can declare
/// it provides.
///
/// The set is deliberately closed and small: an open vocabulary is how
/// "capability" turns into a free-text field nobody validates. A dependency
/// that is not listed is not a capability requirement — it is either provided
/// by the workspace itself (a library, an embedded database such as `sqlite`)
/// or it has to be added here with a keyword list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// A container engine: `docker`, `podman`, `nerdctl`, and compose.
    ContainerRuntime,
    /// A disposable virtual machine: `qemu`, `kvm`, `vagrant`.
    VirtualMachine,
    /// A database server reached over a socket: `postgres`, `mysql`, `redis`.
    Database,
    /// Outbound network to a service the run does not host itself.
    ExternalNetwork,
    /// A device the test drives: CUDA, `nvidia-smi`.
    Gpu,
    /// A real browser: Chromium, Firefox, and their drivers.
    Browser,
}

impl Capability {
    /// Every capability, in wire order.
    pub const ALL: [Self; 6] = [
        Self::ContainerRuntime,
        Self::VirtualMachine,
        Self::Database,
        Self::ExternalNetwork,
        Self::Gpu,
        Self::Browser,
    ];

    /// The stable wire name, as it appears in hold codes and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContainerRuntime => "container-runtime",
            Self::VirtualMachine => "vm",
            Self::Database => "database",
            Self::ExternalNetwork => "external-network",
            Self::Gpu => "gpu",
            Self::Browser => "browser",
        }
    }

    /// Resolves a wire name or a common alias written in an issue body.
    pub fn from_token(token: &str) -> Option<Self> {
        let token = token.trim().to_ascii_lowercase();
        if token.is_empty() {
            return None;
        }
        Self::ALL.into_iter().find(|capability| {
            capability.as_str() == token || capability.aliases().iter().any(|a| *a == token)
        })
    }

    /// Alias spellings accepted by [`Capability::from_token`].
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::ContainerRuntime => &["containers", "docker", "podman", "oci"],
            Self::VirtualMachine => &["virtual-machine", "vms", "qemu", "kvm"],
            Self::Database => &["db", "postgres", "postgresql", "mysql", "databases"],
            Self::ExternalNetwork => &["network", "internet", "egress", "outbound"],
            Self::Gpu => &["gpus", "cuda", "nvidia"],
            Self::Browser => &["browsers", "chromium", "firefox", "playwright"],
        }
    }

    /// Words that mean a criterion is naming this dependency.
    ///
    /// Multi-word entries are matched as phrases; single tokens are matched
    /// with [`token_present`], so `vm` does not fire inside `environment`.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::ContainerRuntime => &[
                "container",
                "containers",
                "docker",
                "podman",
                "nerdctl",
                "compose",
                "oci image",
                "kaniko",
                "buildah",
            ],
            Self::VirtualMachine => &[
                "vm",
                "vms",
                "virtual machine",
                "virtualbox",
                "qemu",
                "kvm",
                "hypervisor",
                "vagrant",
            ],
            Self::Database => &[
                "database",
                "databases",
                "postgres",
                "postgresql",
                "mysql",
                "mariadb",
                "redis",
                "mongodb",
                "database server",
            ],
            Self::ExternalNetwork => &[
                "internet",
                "outbound",
                "external api",
                "third-party api",
                "third party api",
                "live api",
                "public endpoint",
                "network egress",
            ],
            Self::Gpu => &["gpu", "gpus", "cuda", "nvidia", "opencl", "tensor core"],
            Self::Browser => &[
                "browser",
                "browsers",
                "playwright",
                "selenium",
                "chromium",
                "chrome",
                "firefox",
                "chromedriver",
                "headless browser",
            ],
        }
    }

    /// The real binaries that stand in for this capability when it is present.
    ///
    /// These are also the names a substitution fixture takes: a script written
    /// to a path named `docker` is a shim, and [`super::substitution`] flags
    /// files that create one for a capability the criterion names.
    pub fn binaries(self) -> &'static [&'static str] {
        match self {
            Self::ContainerRuntime => &[
                "docker",
                "podman",
                "nerdctl",
                "docker-compose",
                "podman-compose",
                "buildah",
                "skopeo",
                "kubectl",
                "minikube",
                "kind",
            ],
            Self::VirtualMachine => &[
                "qemu-system-x86_64",
                "qemu-img",
                "virt-install",
                "virsh",
                "vagrant",
                "VBoxManage",
            ],
            Self::Database => &[
                "postgres",
                "pg_ctl",
                "psql",
                "mysql",
                "mysqladmin",
                "mariadb",
                "mongod",
                "mongosh",
                "redis-server",
                "redis-cli",
            ],
            Self::ExternalNetwork => &["curl", "wget", "http", "https", "nc"],
            Self::Gpu => &["nvidia-smi", "nvcc"],
            Self::Browser => &[
                "chromium",
                "google-chrome",
                "chrome",
                "firefox",
                "chromedriver",
                "geckodriver",
                "playwright",
            ],
        }
    }

    /// True when `text` (already lowercased) names this dependency.
    fn named_in_lowercase(self, text: &str) -> bool {
        self.keywords()
            .iter()
            .any(|keyword| token_present(text, keyword))
    }
}

/// Every capability a piece of text names, in [`Capability::ALL`] order.
pub fn named_capabilities(text: &str) -> Vec<Capability> {
    let lower = text.to_ascii_lowercase();
    Capability::ALL
        .into_iter()
        .filter(|capability| capability.named_in_lowercase(&lower))
        .collect()
}

/// Which capability a binary name belongs to, if any.
///
/// Case and `-`/`_` spelling are normalised, so `VBoxManage`, `vboxmanage` and
/// `docker_compose` all resolve.
pub fn binary_capability(binary: &str) -> Option<Capability> {
    let needle = binary.trim().to_ascii_lowercase().replace('_', "-");
    Capability::ALL.into_iter().find(|capability| {
        capability
            .binaries()
            .iter()
            .any(|name| name.to_ascii_lowercase().replace('_', "-") == needle)
    })
}

/// What one acceptance criterion, or a whole issue, requires.
///
/// Two sources feed this: an explicit declaration in the issue body (a
/// `## Required capabilities` section written by the decomposer) and the words
/// of the acceptance criteria themselves. Routing uses the **union**, because
/// the criterion is the authority — a task that says "starts a container"
/// needs a container engine whether or not anyone labelled it. The split is
/// kept for reporting: a declared requirement was thought about, an inferred
/// one was caught by the gate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskRequirements {
    declared: BTreeSet<Capability>,
    inferred: BTreeSet<Capability>,
}

impl TaskRequirements {
    /// A requirement set that names nothing external.
    pub fn hermetic() -> Self {
        Self::default()
    }

    /// Requirements stated outright, e.g. parsed from a capability matrix.
    pub fn declared(capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            declared: capabilities.into_iter().collect(),
            inferred: BTreeSet::new(),
        }
    }

    /// Requirements inferred from the text of a single acceptance criterion.
    pub fn from_criterion(text: &str) -> Self {
        Self {
            declared: BTreeSet::new(),
            inferred: named_capabilities(text).into_iter().collect(),
        }
    }

    /// Requirements read out of an issue body.
    ///
    /// The declaration is read from a `## Required capabilities` (or
    /// `## Required dependencies`) section; inference runs over the
    /// `## Acceptance criteria` section only. The rest of the body is prose
    /// about the incident, the motivation, or a pasted log, and keyword
    /// matching over prose turns every mention into a requirement.
    pub fn from_issue_body(body: &str) -> Self {
        let declared = section(body, &["required capabilities", "required dependencies"])
            .iter()
            .flat_map(|section| section.lines())
            .flat_map(parse_declared_line)
            .collect();
        let inferred = section(body, &["acceptance criteria", "acceptance criterion"]).join("\n");
        Self {
            declared,
            inferred: named_capabilities(&inferred).into_iter().collect(),
        }
    }

    /// Every requirement, declared or inferred, deduplicated and sorted.
    pub fn all(&self) -> Vec<Capability> {
        self.declared.union(&self.inferred).copied().collect()
    }

    /// Requirements named by an explicit declaration, as opposed to inferred
    /// from prose. The builder that sets them is [`TaskRequirements::declared`].
    pub fn explicit(&self) -> Vec<Capability> {
        self.declared.iter().copied().collect()
    }

    /// Requirements caught from the criterion text alone.
    pub fn inferred(&self) -> Vec<Capability> {
        self.inferred.iter().copied().collect()
    }

    /// True when nothing external is needed and any executor can run it.
    pub fn is_hermetic(&self) -> bool {
        self.declared.is_empty() && self.inferred.is_empty()
    }

    /// True when the task needs a container engine.
    pub fn requires_container_runtime(&self) -> bool {
        self.all().contains(&Capability::ContainerRuntime)
    }

    /// The requirements `available` does not provide. A hermetic task requires
    /// nothing, so nothing is ever missing from it.
    pub fn missing_in(&self, available: &BTreeSet<Capability>) -> Vec<Capability> {
        if self.is_hermetic() {
            return Vec::new();
        }
        self.all()
            .into_iter()
            .filter(|capability| !available.contains(capability))
            .collect()
    }
}

/// The section bodies under any heading whose text matches one of `titles`.
///
/// Headings are matched case-insensitively after stripping `#` markers and a
/// leading list bullet, so `## Acceptance criteria` and `### Required
/// capabilities (executor)` both resolve. A section ends at the next heading
/// of any level.
fn section(body: &str, titles: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix('#') {
            if let Some(lines) = current.take() {
                found.push(lines.join("\n"));
            }
            let heading = title.trim_start_matches('#').trim().to_ascii_lowercase();
            if titles.iter().any(|wanted| heading.starts_with(*wanted)) {
                current = Some(Vec::new());
            }
            continue;
        }
        if let Some(lines) = current.as_mut() {
            lines.push(line.to_string());
        }
    }
    if let Some(lines) = current {
        found.push(lines.join("\n"));
    }
    found
}

/// Capability tokens from one declaration line.
///
/// `- [ ] container-runtime` and `docker, compose` both yield
/// [`Capability::ContainerRuntime`]; `none` yields nothing, which is how an
/// issue declares itself hermetic.
fn parse_declared_line(line: &str) -> Vec<Capability> {
    let lowered = strip_bullet(line).to_ascii_lowercase();
    if lowered.is_empty()
        || lowered.starts_with("none")
        || lowered.starts_with("no external")
        || lowered == "hermetic"
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    for token in lowered.split([',', '/', ';', '(', ')']) {
        let token = token.trim();
        if let Some(capability) = Capability::from_token(token) {
            out.push(capability);
        }
    }
    if out.is_empty() {
        out = named_capabilities(&lowered);
    }
    out
}

/// `- `, `* ` and `- [ ] ` / `* [x] ` prefixes, then trimmed.
fn strip_bullet(line: &str) -> &str {
    let line = line.trim();
    let line = line.trim_start_matches(['-', '*', '+']);
    let line = line.trim_start();
    match line.strip_prefix('[').and_then(|rest| rest.find(']')) {
        // `index` counts from the stripped `rest`, so the `]` sits at `index + 1`
        // in `line` and the text after it starts one further on.
        Some(index) => line[(index + 2).min(line.len())..].trim_start(),
        None => line,
    }
}

/// What one executor is able to provide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorCapabilities {
    executor_id: String,
    capabilities: BTreeSet<Capability>,
}

impl ExecutorCapabilities {
    /// Declare an executor by id and the capabilities it provides.
    pub fn new(
        executor_id: impl Into<String>,
        capabilities: impl IntoIterator<Item = Capability>,
    ) -> Self {
        Self {
            executor_id: executor_id.into(),
            capabilities: capabilities.into_iter().collect(),
        }
    }

    /// An executor with no external dependency available — a plain CI runner
    /// or a laptop agent with no container engine.
    pub fn hermetic(executor_id: impl Into<String>) -> Self {
        Self::new(executor_id, [])
    }

    /// The executor id as registered.
    pub fn executor_id(&self) -> &str {
        &self.executor_id
    }

    /// The capabilities this executor provides.
    pub fn capabilities(&self) -> Vec<Capability> {
        self.capabilities.iter().copied().collect()
    }

    /// True when `capability` is available on this executor.
    pub fn provides(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Requirements this executor cannot meet.
    pub fn missing(&self, requirements: &TaskRequirements) -> Vec<Capability> {
        requirements
            .all()
            .into_iter()
            .filter(|capability| !self.provides(*capability))
            .collect()
    }

    /// True when this executor can run the task as specified.
    pub fn can_run(&self, requirements: &TaskRequirements) -> bool {
        self.missing(requirements).is_empty()
    }
}

/// Where a task goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routing {
    /// An executor provides every requirement; dispatch to it.
    Dispatch {
        /// The chosen executor.
        executor_id: String,
    },
    /// No executor provides every requirement; hold and keep the task queued.
    Hold(CapabilityUnavailable),
}

impl Routing {
    /// True when the task may be dispatched.
    pub fn dispatches(&self) -> bool {
        matches!(self, Self::Dispatch { .. })
    }

    /// The chosen executor, when there is one.
    pub fn executor_id(&self) -> Option<&str> {
        match self {
            Self::Dispatch { executor_id } => Some(executor_id),
            Self::Hold(_) => None,
        }
    }

    /// The hold reason, when the task is held.
    pub fn hold(&self) -> Option<&CapabilityUnavailable> {
        match self {
            Self::Dispatch { .. } => None,
            Self::Hold(hold) => Some(hold),
        }
    }
}

/// A task that cannot run anywhere in the current executor pool.
///
/// This is a *hold*, not a failure: the task keeps its queue position and its
/// dependency, and the report names what is missing so the operator can add an
/// executor rather than re-run the same incapable one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityUnavailable {
    required: Vec<Capability>,
    considered: Vec<String>,
}

impl CapabilityUnavailable {
    /// The wire code carried in the verdict and the monitor log.
    pub const CODE: &'static str = "CAPABILITY-UNAVAILABLE";

    /// Build a hold for `required` over the executor ids that were considered.
    pub fn new(required: impl IntoIterator<Item = Capability>, considered: &[String]) -> Self {
        let mut required: Vec<Capability> = required.into_iter().collect();
        required.sort();
        required.dedup();
        Self {
            required,
            considered: considered.to_vec(),
        }
    }

    /// The code string, for callers holding a reference.
    pub fn code(&self) -> &'static str {
        Self::CODE
    }

    /// Capabilities no considered executor provides.
    pub fn missing(&self) -> &[Capability] {
        &self.required
    }

    /// Executors that were ruled out.
    pub fn considered(&self) -> &[String] {
        &self.considered
    }

    /// True when at least one capability is genuinely missing; a hold with an
    /// empty list means the pool itself was empty.
    pub fn unsatisfied(&self) -> bool {
        !self.required.is_empty()
    }

    /// One-line report, code first, so a scanner grepping for the code finds
    /// the line that explains it.
    pub fn report(&self) -> String {
        let missing = if self.required.is_empty() {
            "none-registered".to_string()
        } else {
            self.required
                .iter()
                .map(|capability| capability.as_str())
                .collect::<Vec<_>>()
                .join(",")
        };
        let considered = if self.considered.is_empty() {
            "none".to_string()
        } else {
            self.considered.join(",")
        };
        format!(
            "{code} missing={missing} considered={considered} task-stays-queued",
            code = self.code()
        )
    }
}

/// Choose an executor for `requirements`, or hold.
///
/// The first executor in the caller's preference order that provides every
/// requirement wins; preference never overrides capability. With no capable
/// executor, `missing` is the set no executor in the pool provides — the
/// distinction between "one executor is short one thing" and "nothing here can
/// ever run this" is visible in that list.
pub fn route(requirements: &TaskRequirements, executors: &[ExecutorCapabilities]) -> Routing {
    for executor in executors {
        if executor.can_run(requirements) {
            return Routing::Dispatch {
                executor_id: executor.executor_id().to_string(),
            };
        }
    }
    let required = missing_across(requirements, executors);
    Routing::Hold(CapabilityUnavailable::new(
        required,
        &executors
            .iter()
            .map(|executor| executor.executor_id().to_string())
            .collect::<Vec<_>>(),
    ))
}

/// Requirements no executor in `executors` provides.
fn missing_across(
    requirements: &TaskRequirements,
    executors: &[ExecutorCapabilities],
) -> Vec<Capability> {
    let provided: BTreeSet<Capability> = executors
        .iter()
        .flat_map(|executor| executor.capabilities.iter().copied())
        .collect();
    requirements
        .all()
        .into_iter()
        .filter(|capability| !provided.contains(capability))
        .collect()
}

/// The dispatch-time guard for a single already-chosen executor.
///
/// Routing picks an executor; this is the assertion made at the moment of
/// hand-off, so an executor that lost its runtime between selection and
/// dispatch is caught instead of quietly running a fake.
pub fn authorize_dispatch(
    executor: &ExecutorCapabilities,
    requirements: &TaskRequirements,
) -> Result<(), CapabilityUnavailable> {
    let missing = executor.missing(requirements);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CapabilityUnavailable::new(
            missing,
            &[executor.executor_id().to_string()],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executor(id: &str, capabilities: &[Capability]) -> ExecutorCapabilities {
        ExecutorCapabilities::new(id, capabilities.iter().copied())
    }

    fn container_criterion() -> TaskRequirements {
        TaskRequirements::from_criterion(
            "A fresh container starts the compose stack and the probe answers 200.",
        )
    }

    #[test]
    fn criterion_text_names_the_dependency_it_needs() {
        assert_eq!(
            container_criterion().all(),
            vec![Capability::ContainerRuntime]
        );
        assert!(container_criterion().requires_container_runtime());
        assert!(!TaskRequirements::hermetic().requires_container_runtime());
    }

    #[test]
    fn keyword_matching_is_token_bounded() {
        // "environment" is not a virtual machine; "containership" is not a
        // container.
        assert!(
            named_capabilities("provision a vm for the run").contains(&Capability::VirtualMachine)
        );
        assert!(!named_capabilities("the test environment is reproducible")
            .contains(&Capability::VirtualMachine));
        assert!(!named_capabilities("the containership of the artefact")
            .contains(&Capability::ContainerRuntime));
    }

    #[test]
    fn wire_names_round_trip_through_from_token() {
        for capability in Capability::ALL {
            assert_eq!(
                Capability::from_token(capability.as_str()),
                Some(capability)
            );
        }
        assert_eq!(
            Capability::from_token("docker"),
            Some(Capability::ContainerRuntime)
        );
        assert_eq!(Capability::from_token("sqlite"), None);
        assert_eq!(Capability::from_token(""), None);
    }

    #[test]
    fn binary_lookup_normalises_case_and_separators() {
        assert_eq!(
            binary_capability("docker"),
            Some(Capability::ContainerRuntime)
        );
        assert_eq!(
            binary_capability("Docker-Compose"),
            Some(Capability::ContainerRuntime)
        );
        assert_eq!(binary_capability("nvidia-smi"), Some(Capability::Gpu));
        assert_eq!(binary_capability("cargo"), None);
    }

    #[test]
    fn task_without_requirements_dispatches_to_any_executor() {
        let executors = vec![executor("exec-laptop", &[])];
        let routing = route(&TaskRequirements::hermetic(), &executors);
        assert_eq!(
            routing,
            Routing::Dispatch {
                executor_id: "exec-laptop".to_string()
            }
        );
    }

    #[test]
    fn a_container_task_is_never_sent_to_an_executor_without_a_runtime() {
        // The incident: this pool is free, idle, and cannot run a container.
        let executors = vec![
            executor("exec-laptop", &[]),
            executor("exec-ci", &[Capability::Database]),
        ];
        let routing = route(&container_criterion(), &executors);

        assert!(!routing.dispatches());
        assert_eq!(routing.executor_id(), None);
        let hold = routing.hold().expect("held, not dispatched");
        assert_eq!(hold.code(), CapabilityUnavailable::CODE);
        assert_eq!(hold.missing(), &[Capability::ContainerRuntime]);
        assert!(hold.unsatisfied());
        assert_eq!(
            hold.report(),
            "CAPABILITY-UNAVAILABLE missing=container-runtime considered=exec-laptop,exec-ci task-stays-queued"
        );
    }

    #[test]
    fn hold_lists_only_what_nothing_in_the_pool_provides() {
        // Two requirements, one capability shared by the pool, one by nobody:
        // the hold names the gap that matters.
        let requirements =
            TaskRequirements::declared([Capability::ContainerRuntime, Capability::Database]);
        let executors = vec![
            executor("exec-db", &[Capability::Database]),
            executor("exec-db-2", &[Capability::Database, Capability::Gpu]),
        ];
        let hold = route(&requirements, &executors)
            .hold()
            .expect("no executor has a container runtime")
            .clone();
        assert_eq!(hold.missing(), &[Capability::ContainerRuntime]);
        assert_eq!(hold.considered().len(), 2);
    }

    #[test]
    fn an_empty_pool_holds_with_no_missing_capability() {
        let hold = route(&container_criterion(), &[])
            .hold()
            .expect("nothing to dispatch to")
            .clone();
        assert_eq!(hold.missing(), &[Capability::ContainerRuntime]);
        assert!(hold.considered().is_empty());
        assert!(hold.report().contains("considered=none"));
    }

    #[test]
    fn a_capable_executor_wins_over_a_preference_order_that_cannot_run_it() {
        let executors = vec![
            executor("exec-ci", &[]),
            executor(
                "exec-gpu-box",
                &[Capability::ContainerRuntime, Capability::Gpu],
            ),
        ];
        assert_eq!(
            route(&container_criterion(), &executors).executor_id(),
            Some("exec-gpu-box")
        );
    }

    #[test]
    fn dispatch_authorization_catches_a_runtime_lost_after_selection() {
        let requirements = container_criterion();
        let chosen = ExecutorCapabilities::hermetic("exec-gpu-box");
        let error = authorize_dispatch(&chosen, &requirements)
            .expect_err("the runtime disappeared, so dispatch is refused");
        assert_eq!(error.code(), CapabilityUnavailable::CODE);
        assert_eq!(error.missing(), &[Capability::ContainerRuntime]);
        assert_eq!(error.considered(), &["exec-gpu-box".to_string()]);

        let restored = executor("exec-gpu-box", &[Capability::ContainerRuntime]);
        assert!(authorize_dispatch(&restored, &requirements).is_ok());
    }

    #[test]
    fn declared_and_inferred_requirements_union_for_routing() {
        let body = concat!(
            "# Issue\n\nMotivation: the docker incident showed nothing.\n\n",
            "## Required capabilities\n\n",
            "- [ ] container-runtime\n\n",
            "## Acceptance criteria\n\n",
            "- [ ] the suite exits 0\n\n",
            "## Notes\n\n",
            "We mentioned postgres once in a log paste.\n",
        );
        let requirements = TaskRequirements::from_issue_body(body);
        assert_eq!(requirements.explicit(), vec![Capability::ContainerRuntime]);
        assert!(requirements.inferred().is_empty());
        assert!(requirements.requires_container_runtime());
    }

    #[test]
    fn inference_scans_acceptance_criteria_not_prose() {
        let body = concat!(
            "## Goal\n\nReproduce the run where a fake docker binary was accepted.\n\n",
            "## Acceptance criteria\n\n",
            "- [ ] the runner reports hermetic evidence\n",
            "- [ ] a real browser starts\n",
        );
        let requirements = TaskRequirements::from_issue_body(body);
        assert_eq!(requirements.inferred(), vec![Capability::Browser]);
        assert!(!requirements.explicit().contains(&Capability::Browser));
    }

    #[test]
    fn a_declared_none_keeps_the_task_hermetic() {
        let body = "## Required capabilities\n\nnone\n\n## Acceptance criteria\n\n- [ ] exits 0\n";
        assert!(TaskRequirements::from_issue_body(body).is_hermetic());
    }
}
