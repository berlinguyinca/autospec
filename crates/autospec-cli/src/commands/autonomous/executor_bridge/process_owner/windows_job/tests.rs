use super::command_line::{command_line, environment_block};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;

fn rendered(argv: &[&str]) -> String {
    let argv = argv.iter().map(OsString::from).collect::<Vec<_>>();
    let encoded = command_line(&argv).expect("encode Windows command line");
    String::from_utf16(&encoded[..encoded.len() - 1]).expect("decode command line")
}

#[test]
fn command_line_quotes_windows_argv_edge_cases() {
    let cases = [
        (vec![""], "\"\""),
        (vec!["a b"], "\"a b\""),
        (vec!["a\\b"], "a\\b"),
        (vec!["a\"b"], "\"a\\\"b\""),
        (vec!["a b\\"], "\"a b\\\\\""),
        (vec!["a\\\"b"], "\"a\\\\\\\"b\""),
    ];
    for (argv, expected) in cases {
        assert_eq!(rendered(&argv), expected, "argv: {argv:?}");
    }
}

#[test]
fn command_line_rejects_embedded_nul() {
    let program = OsString::from_wide(&[b'a' as u16, 0, b'b' as u16]);
    assert_eq!(
        command_line(&[program]).unwrap_err(),
        "autonomous child command contains an embedded NUL"
    );
}

#[test]
fn empty_environment_block_is_double_nul_terminated() {
    assert_eq!(environment_block(&[]).unwrap(), [0, 0]);
}
