use std::path::Path;

use autospec_core::autonomous_lifecycle::StopMode;

pub(super) fn name(mode: StopMode) -> &'static str {
    match mode {
        StopMode::Graceful => "graceful",
        StopMode::Immediate => "immediate",
    }
}

pub(super) fn print_blocked_start(mode: &str, sentinel: &Path) {
    eprintln!(
        "autospec autonomous start: refusing to launch -- a pending {mode} stop sentinel at {} \
         is still in effect. The previous conductor may also still be draining. Clear it with \
         `autospec-autonomous restart`, or remove that file once the conductor has exited.",
        sentinel.display()
    );
}
