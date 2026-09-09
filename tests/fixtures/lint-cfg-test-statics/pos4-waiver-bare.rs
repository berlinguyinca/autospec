// linter:allow-CFG_TEST_STATIC
#[cfg(test)]
static PROBE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
