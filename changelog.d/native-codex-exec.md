### Changed

- Autospec now drives Codex natively: the Rust drain, the run/verify drain scripts and the
  installer's scanner shims launch `codex exec --cd <dir>
  --dangerously-bypass-approvals-and-sandbox` instead of `omx exec`. The installer no longer
  installs or refreshes oh-my-codex, oh-my-claudecode or oh-my-opencode, and `ensure-tool.sh`
  no longer carries `omx`, `omc` or `oh-my-opencode` install cases.
