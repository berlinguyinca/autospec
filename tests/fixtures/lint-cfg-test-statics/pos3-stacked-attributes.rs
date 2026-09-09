#[cfg(test)]
// armed by a sibling test thread
static RACE: std::cell::RefCell<std::path::PathBuf> = std::cell::RefCell::new(std::path::PathBuf::new());
