Final quality gate
cargo clippy --workspace --all-targets -- -D warnings
FINAL_QUALITY_GATE_FAILED
`crate`, `file`, `line`, and `rule` fields
Do NOT run `gh pr merge` while the final quality gate is failing
