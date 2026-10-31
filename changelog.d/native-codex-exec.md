### Changed

- Autospec now drives Codex natively: the Rust drain, the run/verify drain scripts and the
  installer's scanner shims launch `codex exec --cd <dir>
  --dangerously-bypass-approvals-and-sandbox` instead of `omx exec`. The installer no longer
  installs or refreshes oh-my-codex, oh-my-claudecode or oh-my-opencode, and `ensure-tool.sh`
  no longer carries `omx`, `omc` or `oh-my-opencode` install cases.
- `classify-language.sh` and `classify-model-fit.sh` no longer call `omc ask`; classification is purely deterministic (the `LLM_ESCALATION_THRESHOLD` and `AUTOSPEC_FORCE_LLM` knobs are gone, and an unresolved language tie abstains).
