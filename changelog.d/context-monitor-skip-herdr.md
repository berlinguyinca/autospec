### Added

- The context monitor now skips Herdr-managed sessions (non-empty `HERDR_PANE_ID`): it logs `{"event": "herdr_managed"}` and exits 0 before touching tmux, since Herdr has its own handoff controller. Set `AUTOSPEC_CONTEXT_MONITOR_SKIP_HERDR=0` to opt out; `no-auto-rollover.flag` is unchanged and `doctor` reports the setting. `autospec-session` prints a stderr line at launch when the monitor it starts will skip itself.
