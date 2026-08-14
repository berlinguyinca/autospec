use super::super::*;

#[test]
fn trusted_inherited_launch_preserves_explicit_credentials() {
    let spec = PreparedLaunchSpec::inherited(
        "trusted-program".into(),
        vec!["trusted-program".into()],
        None,
        vec![("GH_TOKEN".into(), "trusted-token".into())],
        None,
        None,
        None,
    );

    assert!(spec
        .environment
        .variables
        .contains(&("GH_TOKEN".into(), "trusted-token".into())));
}

#[test]
fn credentialless_launch_rejects_sensitive_overrides() {
    let result = PreparedLaunchSpec::credentialless(
        "untrusted-program".into(),
        vec!["untrusted-program".into()],
        None,
        vec![("GH_TOKEN".into(), "forbidden-token".into())],
        std::env::temp_dir().join("unused-credentialless-config"),
        false,
        None,
        None,
        None,
    );
    let error = match result {
        Ok(_) => panic!("sensitive override must be rejected"),
        Err(error) => error,
    };

    assert!(
        error.contains("may not restore credential authority"),
        "{error}"
    );
}

#[test]
fn credentialless_launch_replaces_forced_environment_values() {
    #[cfg(windows)]
    let prompt_key = "git_terminal_prompt";
    #[cfg(not(windows))]
    let prompt_key = "GIT_TERMINAL_PROMPT";
    let config = std::fs::canonicalize(std::env::temp_dir())
        .expect("canonical temp directory")
        .join(format!(
            "autospec-credentialless-environment-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
    let _ = std::fs::remove_dir_all(&config);
    let spec = PreparedLaunchSpec::credentialless(
        "untrusted-program".into(),
        vec!["untrusted-program".into()],
        None,
        vec![(prompt_key.into(), "unsafe".into())],
        config.clone(),
        false,
        None,
        None,
        None,
    )
    .expect("prepare credentialless launch");

    let prompts = spec
        .environment
        .variables
        .iter()
        .filter(|(key, _)| environment_keys_match(key, &"GIT_TERMINAL_PROMPT".into()))
        .collect::<Vec<_>>();
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].1, "0");
    std::fs::remove_dir_all(config).expect("remove credentialless config fixture");
}
