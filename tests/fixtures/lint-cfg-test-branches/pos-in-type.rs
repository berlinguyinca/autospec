pub struct Probe {
    live: bool,
    #[cfg(test)]
    test_hook: i32,
}
