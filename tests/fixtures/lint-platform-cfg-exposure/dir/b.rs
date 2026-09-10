#[cfg(not(target_os = "linux"))]
fn b1() {}
#[cfg(not(target_os = "freebsd"))]
fn b2() {}
