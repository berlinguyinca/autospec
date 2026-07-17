use std::path::Path;

use autospec_core::runtime_env::{MavenArgPlatform, MavenArgs, MavenPlan, MavenPurgeTarget};

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
    let posix = MavenArgs::parse_for(
        r#"-T 2 -s '/tmp/settings with spaces.xml' "can't""#,
        MavenArgPlatform::Posix,
    )
    .unwrap();
    assert_eq!(
        posix.render_for(MavenArgPlatform::Posix).unwrap(),
        r#"-T 2 -s '/tmp/settings with spaces.xml' 'can'\''t'"#
    );

    let windows = MavenArgs::parse_for(
        r#"-T 2 -s "C:\Users\Agent Data\settings.xml" "say \"hello\"""#,
        MavenArgPlatform::Windows,
    )
    .unwrap();
    assert_eq!(
        token_strings(&windows),
        [
            "-T",
            "2",
            "-s",
            r#"C:\Users\Agent Data\settings.xml"#,
            r#"say "hello""#,
        ]
    );
    assert_eq!(
        windows.render_for(MavenArgPlatform::Windows).unwrap(),
        r#"-T 2 -s "C:\Users\Agent Data\settings.xml" "say \"hello\"""#
    );
}

#[test]
fn windows_render_doubles_trailing_backslashes_before_the_closing_quote() {
    let args = MavenArgs::from_tokens([r#"C:\Agent Data\"#]);

    assert_eq!(
        args.render_for(MavenArgPlatform::Windows).unwrap(),
        r#""C:\Agent Data\\""#
    );
}

#[test]
fn windows_render_quotes_cmd_metacharacters_before_maven_cmd_expands_them() {
    let args = MavenArgs::from_tokens([
        "left&right",
        "left|right",
        "left^right",
        "(group)",
        "input<output",
        "input>output",
        "-Dservice.url=https://example.test/path?left=1&right=2",
    ]);

    assert_eq!(
        args.render_for(MavenArgPlatform::Windows).unwrap(),
        r#""left&right" "left|right" "left^right" "(group)" "input<output" "input>output" "-Dservice.url=https://example.test/path?left=1&right=2""#
    );
}

#[test]
fn windows_render_rejects_percent_and_bang_expansion() {
    for token in ["%TEMP%", "!MAVEN_SECRET!"] {
        let error = MavenArgs::from_tokens([token])
            .render_for(MavenArgPlatform::Windows)
            .unwrap_err();

        assert_eq!(error.code, "MAVEN_ARGUMENT_UNSAFE_WINDOWS_EXPANSION");
        assert_eq!(error.resource, "MAVEN_ARGS");
    }
}

#[test]
fn posix_double_quotes_apply_only_documented_backslash_escapes() {
    let args = MavenArgs::parse_for(
        r#""\$HOME \`date\` \\ \" \q line\
continued""#,
        MavenArgPlatform::Posix,
    )
    .unwrap();

    assert_eq!(
        token_strings(&args),
        ["$HOME `date` \\ \" \\q linecontinued"]
    );
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
fn maven_arguments_accept_equal_separated_system_properties() {
    let args = MavenPlan::arguments(
        "-D aether.lrm.enhanced.remotePrefix=cached -T 2",
        "sample-a",
    )
    .unwrap();

    assert_eq!(
        token_strings(&args),
        [
            "-T",
            "2",
            "-Daether.lrm.enhanced.split=true",
            "-Daether.lrm.enhanced.remotePrefix=cached",
            "-Daether.lrm.enhanced.localPrefix=autospec/sample-a",
            "-Daether.system.named.factory=file-lock",
        ]
    );
}

#[test]
fn maven_arguments_reject_conflicting_separated_system_properties() {
    let error = MavenPlan::arguments("-D aether.system.named.factory=rwlock-local", "sample-a")
        .unwrap_err();

    assert_eq!(error.code, "MAVEN_ARGUMENT_CONFLICT");
    assert_eq!(error.resource, "aether.system.named.factory");
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

fn token_strings(arguments: &MavenArgs) -> Vec<String> {
    arguments
        .tokens()
        .iter()
        .map(|token| token.to_string_lossy().into_owned())
        .collect()
}
