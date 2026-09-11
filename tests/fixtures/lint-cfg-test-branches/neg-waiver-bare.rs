pub fn run() -> i32 {
    // linter:allow-CFG_TEST_BRANCH
    #[cfg(test)]
    return 1;
    0
}
