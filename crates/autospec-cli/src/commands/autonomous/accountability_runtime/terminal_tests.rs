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
fn terminal_record_failure_still_releases_but_suppresses_output() {
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
    assert_eq!(*order.borrow(), ["record", "release"]);
}
