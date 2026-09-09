use std::sync::Mutex;

#[cfg(test)]
static FORK_LIFECYCLE: Mutex<()> = Mutex::new(());
