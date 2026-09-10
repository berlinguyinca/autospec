#[cfg(not(target_os = "linux"))]
fn portable() {}
#[cfg(all(test, not(target_os = "linux")))]
fn portable_test() {}
#[cfg(not(target_os = "windows"))]
fn not_windows() {}
#[cfg(target_os = "linux")]
fn linux_only() {}
