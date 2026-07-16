use autospec_core::autonomous::config::{AutonomousConfig, Tier4Config, Tier4SourceDescriptor};

#[test]
fn parses_one_source_with_exact_descriptor_values() {
    let config = AutonomousConfig::parse(
        "tier4:\n  sources:\n    - id: release-feed\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n",
    )
    .expect("one valid Tier 4 source parses");

    assert_eq!(
        config.tier4,
        Tier4Config {
            sources: vec![Tier4SourceDescriptor {
                id: "release-feed".to_string(),
                host: "api.example.test".to_string(),
                path: "/v1/releases".to_string(),
                max_bytes: 65_536,
                deadline_millis: 5_000,
            }],
        }
    );
}

#[test]
fn parses_two_sources_in_checked_in_order() {
    let config = AutonomousConfig::parse(
        "tier4:\n  sources:\n    - id: release-feed\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n    - id: docs-index\n      host: docs.example.test\n      path: /guide/index\n      max_bytes: 1048576\n      deadline_millis: 30000\n",
    )
    .expect("two valid Tier 4 sources parse");

    assert_eq!(
        config.tier4.sources,
        vec![
            Tier4SourceDescriptor {
                id: "release-feed".to_string(),
                host: "api.example.test".to_string(),
                path: "/v1/releases".to_string(),
                max_bytes: 65_536,
                deadline_millis: 5_000,
            },
            Tier4SourceDescriptor {
                id: "docs-index".to_string(),
                host: "docs.example.test".to_string(),
                path: "/guide/index".to_string(),
                max_bytes: 1_048_576,
                deadline_millis: 30_000,
            },
        ]
    );
}

#[test]
fn absence_mixed_main_health_and_nested_unrelated_tier4_are_safe() {
    for (name, source, branch) in [
        ("absent", "", None),
        (
            "mixed with main health",
            "main_health:\n  branch: master_ai\n  ignore_checks:\n    - Unit Tests\ntier4:\n  sources:\n    - id: release-feed\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n",
            Some("master_ai"),
        ),
        (
            "nested unrelated policy",
            "other_policy:\n  tier4:\n    sources:\n      - id: ignored\n        host: ignored.example.test\n        path: /ignored\n        max_bytes: 1\n        deadline_millis: 100\n",
            None,
        ),
    ] {
        let config = AutonomousConfig::parse(source).expect(name);
        assert_eq!(config.main_health.branch.as_deref(), branch, "{name}");
        if name == "mixed with main health" {
            assert_eq!(config.tier4.sources.len(), 1, "{name}");
        } else {
            assert_eq!(config.tier4, Tier4Config::default(), "{name}");
        }
    }
}

#[test]
fn rejects_malformed_relevant_tier4_shapes() {
    let valid = "    - id: release-feed\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n";
    let five_sources = format!(
        "tier4:\n  sources:\n{}{}{}{}{}",
        source("one", "one.example.test"),
        source("two", "two.example.test"),
        source("three", "three.example.test"),
        source("four", "four.example.test"),
        source("five", "five.example.test"),
    );
    let cases = vec![
        ("duplicate blocks", format!("tier4:\n  sources:\n{valid}tier4:\n  sources:\n{valid}")),
        (
            "duplicate field",
            "tier4:\n  sources:\n    - id: release-feed\n      host: api.example.test\n      host: api-two.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n".to_string(),
        ),
        (
            "duplicate id",
            format!(
                "tier4:\n  sources:\n{}{}",
                source("release-feed", "one.example.test"),
                source("release-feed", "two.example.test"),
            ),
        ),
        (
            "duplicate host",
            format!(
                "tier4:\n  sources:\n{}{}",
                source("release-feed", "api.example.test"),
                source("docs-index", "api.example.test"),
            ),
        ),
        ("zero sources", "tier4:\n  sources:\n".to_string()),
        ("five sources", five_sources),
        (
            "tab indentation",
            "tier4:\n\tsources:\n    - id: release-feed\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n".to_string(),
        ),
        (
            "bad indentation",
            "tier4:\n   sources:\n    - id: release-feed\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n".to_string(),
        ),
        ("inline sources", "tier4:\n  sources: []\n".to_string()),
        (
            "inline descriptor field",
            "tier4:\n  sources:\n    - id: [release-feed]\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n".to_string(),
        ),
        (
            "unknown field",
            "tier4:\n  enabled: true\n".to_string(),
        ),
    ];

    assert_all_rejected(cases);
}

#[test]
fn rejects_invalid_source_identifiers_hosts_paths_and_limits() {
    let mut cases = Vec::new();
    for id in [
        "",
        "Release-feed",
        "release_feed",
        "-release",
        "release-",
        "release--feed",
    ] {
        cases.push((format!("invalid id {id:?}"), descriptor_field("id", id)));
    }
    cases.push((
        "too-long id".to_string(),
        descriptor_field("id", &"a".repeat(65)),
    ));

    for host in [
        "API.example.test",
        "127.0.0.1",
        "0x7f.0x0.0x0.0x1",
        "0177.0.0.1",
        "https://api.example.test",
        "api.example.test:443",
        "user@api.example.test",
        "*.example.test",
    ] {
        cases.push((
            format!("invalid host {host:?}"),
            descriptor_field("host", host),
        ));
    }

    for path in [
        "releases",
        "/v1//releases",
        "/v1/./releases",
        "/v1/../releases",
        "/v1?q=1",
        "/v1#top",
        "/v1\\release",
    ] {
        cases.push((
            format!("invalid path {path:?}"),
            descriptor_field("path", path),
        ));
    }

    for (field, value) in [
        ("max_bytes", "+1"),
        ("max_bytes", "-1"),
        ("max_bytes", "one"),
        ("max_bytes", "4294967296"),
        ("max_bytes", "0"),
        ("max_bytes", "1048577"),
        ("deadline_millis", "+100"),
        ("deadline_millis", "-100"),
        ("deadline_millis", "fast"),
        ("deadline_millis", "4294967296"),
        ("deadline_millis", "99"),
        ("deadline_millis", "30001"),
    ] {
        cases.push((
            format!("invalid {field} {value:?}"),
            descriptor_field(field, value),
        ));
    }

    assert_all_rejected(cases);
}

#[test]
fn reports_the_offending_host_and_path_line() {
    for (name, source, line) in [
        (
            "host",
            "tier4:\n  sources:\n    - id: release-feed\n      host: API.example.test\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n",
            4,
        ),
        (
            "path",
            "tier4:\n  sources:\n    - id: release-feed\n      host: api.example.test\n      path: /v1?q=1\n      max_bytes: 65536\n      deadline_millis: 5000\n",
            5,
        ),
    ] {
        let error = AutonomousConfig::parse(source).expect_err(name);
        assert!(
            error.starts_with(&format!("invalid .autospec/autonomous.yml at line {line}:")),
            "{name} must report its field line: {error}"
        );
    }
}

fn source(id: &str, host: &str) -> String {
    format!(
        "    - id: {id}\n      host: {host}\n      path: /v1/releases\n      max_bytes: 65536\n      deadline_millis: 5000\n"
    )
}

fn descriptor_field(field: &str, value: &str) -> String {
    let mut descriptor = Tier4SourceDescriptor {
        id: "release-feed".to_string(),
        host: "api.example.test".to_string(),
        path: "/v1/releases".to_string(),
        max_bytes: 65_536,
        deadline_millis: 5_000,
    };
    match field {
        "id" => descriptor.id = value.to_string(),
        "host" => descriptor.host = value.to_string(),
        "path" => descriptor.path = value.to_string(),
        "max_bytes" => return descriptor_with_numeric(value, "5000"),
        "deadline_millis" => return descriptor_with_numeric("65536", value),
        _ => unreachable!("test only supplies known descriptor fields"),
    }
    format!(
        "tier4:\n  sources:\n    - id: {}\n      host: {}\n      path: {}\n      max_bytes: {}\n      deadline_millis: {}\n",
        descriptor.id, descriptor.host, descriptor.path, descriptor.max_bytes, descriptor.deadline_millis
    )
}

fn descriptor_with_numeric(max_bytes: &str, deadline_millis: &str) -> String {
    format!(
        "tier4:\n  sources:\n    - id: release-feed\n      host: api.example.test\n      path: /v1/releases\n      max_bytes: {max_bytes}\n      deadline_millis: {deadline_millis}\n"
    )
}

fn assert_all_rejected(cases: Vec<(impl AsRef<str> + std::fmt::Debug, String)>) {
    for (name, source) in cases {
        let error = AutonomousConfig::parse(&source).expect_err(name.as_ref());
        assert!(
            error.starts_with("invalid .autospec/autonomous.yml at line "),
            "{name:?} must use the established line-numbered diagnostic: {error}"
        );
    }
}
