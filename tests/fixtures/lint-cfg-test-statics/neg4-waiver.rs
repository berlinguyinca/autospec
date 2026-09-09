// linter:allow-CFG_TEST_STATIC shared fork barrier holds no per-test data
#[cfg(test)]
static PROBE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
