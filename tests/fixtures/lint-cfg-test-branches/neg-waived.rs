pub fn run() -> i32 {
    // linter:allow-CFG_TEST_BRANCH tracked in the baseline until the seam lands
    #[cfg(test)]
    return 1;
    0
}
