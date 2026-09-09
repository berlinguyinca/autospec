use std::sync::atomic::AtomicU8;

#[cfg(test)]
thread_local! {
    static PROBE: std::cell::RefCell<AtomicU8> =
        std::cell::RefCell::new(AtomicU8::new(0));
}
