use autospec_core::validation::{Jobs, ValidationOptions};

#[test]
fn options_accept_fast_scoped_and_parallel_forms() {
    let options = ValidationOptions::parse(["--fast", "--changed=origin/main", "--jobs=4"])
        .expect("validation options parse");

    assert!(options.fast);
    assert_eq!(options.changed_base.as_deref(), Some("origin/main"));
    assert_eq!(options.jobs, Jobs::Fixed(4));
}

#[test]
fn options_accept_aliases_default_scope_and_auto_parallelism() {
    let options = ValidationOptions::parse(["--no-bats", "--changed", "--since", "v1", "--jobs"])
        .expect("validation options parse");

    assert!(options.fast);
    assert_eq!(options.changed_base.as_deref(), Some("origin/main"));
    assert_eq!(options.since.as_deref(), Some("v1"));
    assert_eq!(options.jobs, Jobs::Auto);
}

#[test]
fn options_reject_unknown_or_incomplete_values() {
    assert!(ValidationOptions::parse(["--unknown"]).is_err());
    assert!(ValidationOptions::parse(["--since"]).is_err());
    assert!(ValidationOptions::parse(["--jobs=0"]).is_err());
}

#[test]
fn options_reject_path_and_shadow_results_in_either_order() {
    assert!(
        ValidationOptions::parse(["--path", "src/lib.rs", "--shadow-results", "result.json"])
            .is_err()
    );
    assert!(
        ValidationOptions::parse(["--shadow-results", "result.json", "--path", "src/lib.rs"])
            .is_err()
    );
}

#[test]
fn options_remove_only_the_option_prefix_from_assigned_values() {
    let options =
        ValidationOptions::parse(["--changed=--changed=origin/main"]).expect("changed base parses");

    assert_eq!(
        options.changed_base.as_deref(),
        Some("--changed=origin/main")
    );
    assert!(ValidationOptions::parse(["--jobs=--jobs=4"]).is_err());
}
