use std::sync::atomic::AtomicU8;

#[cfg(test)]
static PROBE: AtomicU8 = AtomicU8::new(0);
