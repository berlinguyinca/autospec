use super::*;
use std::cell::RefCell;

#[test]
fn terminal_accountability_precedes_release_and_output() {
    let order = RefCell::new(Vec::new());
    let result = finish_accountability_boundary(
        "terminal",
        |_| {
            order.borrow_mut().push("record");
            Ok(())
        },
        || {
            order.borrow_mut().push("release");
            Ok(())
        },
        |value| {
            order.borrow_mut().push("emit");
            Ok(value)
        },
    )
    .expect("finish boundary");

    assert_eq!(result, "terminal");
    assert_eq!(*order.borrow(), ["record", "release", "emit"]);
}

#[test]
fn terminal_record_failure_retains_lease_and_suppresses_output() {
    let order = RefCell::new(Vec::new());
    let result = finish_accountability_boundary(
        (),
        |_| {
            order.borrow_mut().push("record");
            Err(CommandFailure::diagnostic("journal failed"))
        },
        || {
            order.borrow_mut().push("release");
            Ok(())
        },
        |_| {
            order.borrow_mut().push("emit");
            Ok(())
        },
    );

    assert!(result.is_err());
    assert_eq!(*order.borrow(), ["record"]);
}

#[test]
fn terminal_projection_must_ack_before_lifecycle_release() {
    assert!(require_terminal_projection_ack(0, None).is_ok());
    let error = require_terminal_projection_ack(1, Some(1234)).unwrap_err();
    assert!(error.message.contains("retained lifecycle ownership"));
    assert!(error.message.contains("1234"));
}

#[test]
fn supervisor_relaunch_requires_active_accountability() {
    assert!(accountability_allows_relaunch(
        accountability::RecoveryState::Active
    ));
    assert!(!accountability_allows_relaunch(
        accountability::RecoveryState::Parked
    ));
    assert!(!accountability_allows_relaunch(
        accountability::RecoveryState::Terminal
    ));
}

#[test]
fn inherited_foreground_early_exits_use_the_accountable_terminal_path() {
    let source = include_str!("../../autonomous.rs");
    let foreground = source
        .split_once("fn run_foreground(options: Options)")
        .expect("run_foreground exists")
        .1
        .split_once("fn run_foreground_cycles(")
        .expect("run_foreground boundary exists")
        .0;

    assert_eq!(
        foreground.matches("finish_foreground_with_lease(").count(),
        4
    );
    assert_eq!(foreground.matches("finish_with_launch_lease(").count(), 1);
}
