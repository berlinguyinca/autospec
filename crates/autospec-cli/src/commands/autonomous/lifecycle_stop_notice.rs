use std::path::Path;

pub(super) fn render(mode: &str, stop_flag: &Path, stopped: usize, draining: bool) -> String {
    let conductor = if draining {
        "conductor is STILL RUNNING and will stop at its next issue/cycle boundary; it is not \
         killed, because terminating it can strand a completed child before the EXIT/DONE fence. \
         Watch `autospec-autonomous status`."
    } else {
        "conductor is not running"
    };
    format!(
        "autospec autonomous stop: mode={mode} stop_flag={} stopped {stopped} companion(s)\n\
         autospec autonomous stop: {conductor}\n",
        stop_flag.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_stop_attributes_the_count_and_names_the_live_conductor() {
        let notice = render("immediate", Path::new("/tmp/stop.flag"), 2, true);

        assert!(notice.contains("stopped 2 companion(s)"));
        assert!(notice.contains("conductor is STILL RUNNING"));
    }
}
