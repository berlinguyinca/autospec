// A `target_os = "linux"` gate is the OPPOSITE of exposure: this code IS
// compiled on the Linux development host, so it needs no flag.
#[cfg(target_os = "linux")]
fn linux_only() {}
#[cfg(all(test, target_os = "linux"))]
fn linux_test() {}
