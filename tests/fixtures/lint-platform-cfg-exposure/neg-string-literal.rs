// A test that splits on a cfg snippet must not be counted as an exposure
// block: the quotes are escaped, so the plain-quote detector skips it.
fn parse(input: &str) -> Vec<&str> {
    input
        .split("#[cfg(not(target_os = \"linux\"))]\nfn quarantine")
        .collect()
}
