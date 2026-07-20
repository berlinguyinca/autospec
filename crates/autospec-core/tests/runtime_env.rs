use autospec_core::runtime_env::{
    ComposeIsolation, ExportProtocol, ExportValue, MavenIsolation, RuntimeContext, RuntimeManifest,
    RuntimeState,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_TEMP_REPO: AtomicUsize = AtomicUsize::new(0);

struct TempRepo {
    root: PathBuf,
}

impl TempRepo {
    fn with_files(files: &[(&str, &str)]) -> Self {
        let suffix = NEXT_TEMP_REPO.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "autospec-runtime-env-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (relative, content) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().expect("fixture file has parent"))
                .expect("create fixture parent");
            std::fs::write(path, content).expect("write fixture file");
        }
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const VALID_AUTOSPEC_MANIFEST: &str = "version: 1\nname: sample-app\ndefault_mode: e2e-local-db\nmodes:\n  e2e-local-db:\n    env:\n      E2E_USE_HARNESS: \"1\"\n      QUOTED_VALUE: \"it's-safe\"\n    command: sh -c 'true'\n";
const VALID_AGENT_MANIFEST: &str = "version: 1\nname: fallback\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n";

#[test]
fn manifest_prefers_autospec_path_and_preserves_mode_order() {
    let fixture = TempRepo::with_files(&[
        (".autospec/runtime.yml", VALID_AUTOSPEC_MANIFEST),
        (".agent-runtime.yml", VALID_AGENT_MANIFEST),
    ]);

    let manifest = RuntimeManifest::read_from_repo(fixture.path()).expect("manifest reads");

    assert_eq!(
        manifest.path(),
        fixture.path().join(".autospec/runtime.yml")
    );
    assert_eq!(
        manifest.selected_mode("auto").expect("default mode").name(),
        "e2e-local-db"
    );
}

#[cfg(unix)]
#[test]
fn context_keeps_the_selected_manifest_path_when_it_is_a_symlink() {
    let fixture = TempRepo::with_files(&[("manifest-source.yml", VALID_AUTOSPEC_MANIFEST)]);
    let manifest_dir = fixture.path().join(".autospec");
    std::fs::create_dir_all(&manifest_dir).expect("create manifest directory");
    let selected_path = manifest_dir.join("runtime.yml");
    std::os::unix::fs::symlink("../manifest-source.yml", &selected_path)
        .expect("create manifest symlink");

    let manifest = RuntimeManifest::read_from_repo(fixture.path()).expect("manifest reads");
    let context = RuntimeContext::new(
        manifest,
        fixture.path(),
        "auto",
        &fixture.path().join("state"),
    )
    .expect("context builds");
    let state = RuntimeState::from_context(&context, 41001, 41002);
    let expected_path = std::fs::canonicalize(fixture.path())
        .expect("canonical repo")
        .join(".autospec/runtime.yml");

    assert_eq!(context.manifest.path(), expected_path);
    assert_eq!(state.value("AGENT_ENV_MANIFEST"), expected_path.to_str());
}

#[test]
fn manifest_rejects_lowercase_environment_names() {
    let error =
        RuntimeManifest::parse("version: 1\nmodes:\n  local:\n    env:\n      lowercase_port: 1\n")
            .expect_err("invalid environment key is rejected");

    assert!(error.to_string().contains("invalid environment name"));
}

#[test]
fn manifest_rejects_broker_owned_environment_names() {
    for key in [
        "AGENT_ENV_ID",
        "AGENT_ENV_MODE",
        "AGENT_ENV_REPO",
        "AGENT_ENV_MANIFEST",
        "AGENT_FRONTEND_PORT",
        "AGENT_BACKEND_PORT",
        "AGENT_PUBLIC_URL",
        "AUTOSPEC_PUBLIC_URL",
        "COMPOSE_PROJECT_NAME",
    ] {
        let error = RuntimeManifest::parse(&format!(
            "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n    env:\n      {key}: override\n"
        ))
        .expect_err("broker-owned environment key is rejected");

        assert!(
            error
                .to_string()
                .contains("reserved runtime environment name"),
            "unexpected error for {key}: {error}"
        );
    }
}

#[test]
fn manifest_uses_first_declared_mode_when_default_is_absent() {
    let manifest = RuntimeManifest::parse(
        "version: 1\nname: ordered\nmodes:\n  first:\n    command: sh -c 'true'\n  second:\n    command: sh -c 'true'\n",
    )
    .expect("manifest parses");

    assert_eq!(
        manifest.selected_mode("auto").expect("first mode").name(),
        "first"
    );
}

#[test]
fn manifest_rejects_default_mode_that_is_not_declared() {
    let error = RuntimeManifest::parse(
        "version: 1\ndefault_mode: missing\nmodes:\n  local:\n    command: sh -c 'true'\n",
    )
    .expect_err("missing declared default mode is rejected during parsing");

    assert!(
        error
            .to_string()
            .contains("default runtime mode is not declared"),
        "unexpected error: {error}"
    );
}

#[test]
fn companion_runtime_manifest_fixtures_parse_and_select_defaults() {
    for (source, expected_mode) in [
        (
            include_str!("../../../tests/fixtures/runtime-manifests/lc-binbase-scheduler.yml"),
            "playwright-local",
        ),
        (
            include_str!("../../../tests/fixtures/runtime-manifests/companion-stack.yml"),
            "go-modules",
        ),
    ] {
        let manifest = RuntimeManifest::parse(source).expect("fixture parses");
        assert_eq!(
            manifest.selected_mode("auto").expect("default mode").name(),
            expected_mode
        );
    }
}

#[test]
fn companion_runbook_lists_every_broker_owned_environment_key() {
    let runbook = include_str!("../../../docs/runbooks/agent-runtime-companion-stacks.md");

    for key in [
        "AGENT_ENV_ID",
        "AGENT_ENV_MODE",
        "AGENT_ENV_REPO",
        "AGENT_ENV_MANIFEST",
        "AGENT_FRONTEND_PORT",
        "AGENT_BACKEND_PORT",
        "AGENT_PUBLIC_URL",
        "AUTOSPEC_PUBLIC_URL",
        "COMPOSE_PROJECT_NAME",
    ] {
        assert!(
            runbook.contains(key),
            "runbook omits broker-owned key {key}"
        );
    }
}

#[test]
fn manifest_rejects_unknown_versions_and_duplicate_mode_names() {
    let version_error =
        RuntimeManifest::parse("version: 3\nmodes:\n  local:\n    command: sh -c 'true'\n")
            .expect_err("unsupported version is rejected");
    assert!(version_error
        .to_string()
        .contains("unsupported runtime manifest version"));

    let duplicate_error = RuntimeManifest::parse(
        "version: 1\nmodes:\n  local:\n    command: sh -c 'true'\n  local:\n    command: sh -c 'true'\n",
    )
    .expect_err("duplicate mode is rejected");
    assert!(duplicate_error
        .to_string()
        .contains("duplicate runtime mode"));
}

#[test]
fn v2_resources_parse_exports_and_logical_preserved_volumes() {
    let manifest = RuntimeManifest::parse(include_str!(
        "../../../tests/fixtures/runtime-resources/manifest-v2.yml"
    ))
    .expect("version 2 manifest parses");
    let resources = manifest.resources();

    assert_eq!(resources.maven.isolation, MavenIsolation::SplitLocal);
    assert_eq!(resources.compose.isolation, ComposeIsolation::Managed);
    assert_eq!(resources.compose.files, vec![PathBuf::from("compose.yaml")]);
    assert_eq!(resources.compose.exports[0].service, "web");
    assert_eq!(resources.compose.exports[0].target, 8080);
    assert_eq!(resources.compose.exports[0].protocol, ExportProtocol::Http);
    assert_eq!(resources.compose.exports[0].env, "AUTOSPEC_PUBLIC_URL");
    assert_eq!(resources.compose.exports[0].value, ExportValue::Url);
    assert_eq!(resources.compose.preserve_volumes, vec!["postgres-data"]);
    assert_eq!(resources.compose.shared_networks, vec!["developer-proxy"]);
    assert_eq!(resources.compose.shared_volumes, vec!["maven-cache"]);
    assert_eq!(
        manifest.selected_mode("auto").expect("default mode").name(),
        "local"
    );
}

#[test]
fn context_and_state_round_trip_the_shell_environment_contract() {
    let fixture = TempRepo::with_files(&[(".autospec/runtime.yml", VALID_AUTOSPEC_MANIFEST)]);
    let manifest = RuntimeManifest::read_from_repo(fixture.path()).expect("manifest reads");
    let state_root = fixture.path().join("state");
    let context =
        RuntimeContext::new(manifest, fixture.path(), "auto", &state_root).expect("context builds");
    let state = RuntimeState::from_context(&context, 41001, 41002);

    let rendered = state.render_env_file();

    assert!(rendered.contains("export AGENT_ENV_ID='sample-app-"));
    assert!(rendered.contains("export AGENT_ENV_MANIFEST='"));
    assert!(!rendered.contains("export AGENT_ENV_FILE="));
    assert!(rendered.contains("export AGENT_FRONTEND_PORT='41001'"));
    assert!(rendered.contains("export AGENT_BACKEND_PORT='41002'"));
    assert!(rendered.contains("export AUTOSPEC_PUBLIC_URL='http://127.0.0.1:41001'"));
    assert!(rendered.contains("export QUOTED_VALUE='it'\\''s-safe'"));
    assert_eq!(
        RuntimeState::from_env_file(&rendered).expect("state parses"),
        state
    );
}

#[test]
fn state_uses_the_shell_compose_slug_rules() {
    let fixture = TempRepo::with_files(&[(
        ".autospec/runtime.yml",
        "version: 1\nname: A-_B\ndefault_mode: local\nmodes:\n  local:\n    command: sh -c 'true'\n",
    )]);
    let manifest = RuntimeManifest::read_from_repo(fixture.path()).expect("manifest reads");
    let context = RuntimeContext::new(
        manifest,
        fixture.path(),
        "auto",
        &fixture.path().join("state"),
    )
    .expect("context builds");
    let state = RuntimeState::from_context(&context, 41001, 41002);

    assert!(state
        .value("COMPOSE_PROJECT_NAME")
        .expect("compose project name")
        .starts_with("agent_a_b_"));
}

#[test]
fn environment_state_rejects_incomplete_or_executable_env_files() {
    let error = RuntimeState::from_env_file("export AGENT_ENV_ID='only-one'\n")
        .expect_err("incomplete state cannot load");
    assert!(error
        .to_string()
        .contains("missing required environment value"));

    let error = RuntimeState::from_env_file("source ./unsafe\n")
        .expect_err("state files are data, not shell programs");
    assert!(error.to_string().contains("invalid environment file line"));
}
