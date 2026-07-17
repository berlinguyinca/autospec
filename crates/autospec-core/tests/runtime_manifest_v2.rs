use autospec_core::runtime_env::{ExportValue, RuntimeManifest};

#[test]
fn v2_yaml_parsing_accepts_comments_and_quoted_keys_and_values() {
    let manifest = RuntimeManifest::parse(
        "# lead\n\"version\": \"2\" # schema\n\"default_mode\": \"local\"\nmodes:\n  \"local\":\n    command: \"printf ready\" # command\n    env:\n      FEATURE_FLAG: \"enabled\"\nresources:\n  compose:\n    isolation: off\n",
    )
    .expect("valid YAML spelling parses");

    let mode = manifest
        .selected_mode("auto")
        .expect("default mode resolves");
    assert_eq!(mode.name(), "local");
    assert_eq!(mode.command(), Some("printf ready"));
    assert_eq!(mode.env(), &[("FEATURE_FLAG".into(), "enabled".into())]);
}

#[test]
fn v2_yaml_rejects_unknown_nested_mode_and_top_level_keys() {
    for (source, expected) in [
        (
            "version: 2\nmodes:\n  local:\n    unknown: value\n",
            "unknown key in runtime mode",
        ),
        (
            "version: 2\nmodes:\n  local:\n    env:\n      FEATURE_FLAG: yes\n    nested: value\n",
            "unknown key in runtime mode",
        ),
        (
            "version: 2\nunknown: value\n",
            "unknown key in runtime manifest",
        ),
    ] {
        let error = RuntimeManifest::parse(source).expect_err("unknown v2 key is rejected");
        assert!(error.to_string().contains(expected), "{error:?}");
    }
}

#[test]
fn export_protocol_and_value_combinations_are_constrained() {
    for (protocol, value) in [
        ("http", "port"),
        ("https", "port"),
        ("tcp", "url"),
        ("udp", "url"),
    ] {
        let source = format!(
            "version: 2\nresources:\n  compose:\n    exports:\n      - service: web\n        target: 80\n        protocol: {protocol}\n        env: SERVICE_VALUE\n        value: {value}\n"
        );
        let error = RuntimeManifest::parse(&source).expect_err("incompatible export is rejected");
        assert!(error.to_string().contains("incompatible"), "{error:?}");
    }

    let host_port = RuntimeManifest::parse(
        "version: 2\nresources:\n  compose:\n    exports:\n      - service: web\n        target: 80\n        protocol: http\n        env: SERVICE_VALUE\n        value: host-port\n",
    )
    .expect("host-port remains protocol independent");
    assert_eq!(
        host_port.resources().compose.exports[0].value,
        ExportValue::HostPort
    );
}
