use std::path::Path;

use autospec_core::runtime_env::{MavenArgs, MavenPlan, MavenPurgeTarget};

#[test]
fn maven_arguments_share_remote_cache_and_split_local_installs() {
    let args = MavenPlan::arguments("-T 2", "sample-a").unwrap();
    let tokens = args
        .tokens()
        .iter()
        .map(|token| token.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(&tokens[..2], ["-T", "2"]);
    assert_eq!(
        &tokens[2..],
        [
            "-Daether.lrm.enhanced.split=true",
            "-Daether.lrm.enhanced.remotePrefix=cached",
            "-Daether.lrm.enhanced.localPrefix=autospec/sample-a",
            "-Daether.system.named.factory=file-lock",
        ]
    );
}

#[test]
fn maven_arguments_round_trip_quoted_posix_and_windows_tokens() {
    for existing in [
        r#"-T 2 -s '/tmp/settings with spaces.xml'"#,
        r#"-T 2 -s \"C:\\Users\\Agent Data\\settings.xml\""#,
    ] {
        let args = MavenPlan::arguments(existing, "sample-a").unwrap();
        assert_eq!(
            MavenArgs::parse(&args.render()).unwrap().tokens(),
            args.tokens()
        );
    }
}

#[test]
fn maven_arguments_reject_conflicting_managed_properties() {
    let error = MavenPlan::arguments(
        "-Daether.lrm.enhanced.remotePrefix=private-cache",
        "sample-a",
    )
    .unwrap_err();

    assert_eq!(error.code, "MAVEN_ARGUMENT_CONFLICT");
    assert!(error.evidence.contains("aether.lrm.enhanced.remotePrefix"));
}

#[test]
fn maven_arguments_reject_a_conflict_after_an_accepted_duplicate() {
    let error = MavenPlan::arguments(
        "-Daether.lrm.enhanced.split=true -Daether.lrm.enhanced.split=false",
        "sample-a",
    )
    .unwrap_err();

    assert_eq!(error.code, "MAVEN_ARGUMENT_CONFLICT");
}

#[test]
fn maven_purge_rejects_a_prefix_that_escapes_the_effective_repository() {
    let error =
        MavenPurgeTarget::new(Path::new("/m2"), Path::new("/tmp/elsewhere"), "env-a").unwrap_err();

    assert_eq!(error.code, "MAVEN_PURGE_OUTSIDE_REPOSITORY");
}

#[test]
fn maven_purge_rejects_an_environment_identity_with_path_syntax() {
    let error = MavenPurgeTarget::for_environment(Path::new("/m2"), "../cached").unwrap_err();

    assert_eq!(error.code, "MAVEN_PURGE_IDENTITY_MISMATCH");
}
