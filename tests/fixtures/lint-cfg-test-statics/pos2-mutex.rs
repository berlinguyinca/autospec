use std::sync::Mutex;

#[cfg(test)]
static COLLECTOR: Mutex<Vec<u32>> = Mutex::new(Vec::new());
