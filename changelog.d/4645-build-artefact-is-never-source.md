# 4645 — a build artefact committed as source is rejected before review

An 8.2 MB compiled ELF was committed and merged in `inferweave-gateway` because
`go build ./cmd/x` wrote the executable into the tree and `git add -A` swept it
in; nothing rejected it and review cannot read a diff of an 8 MB binary.

The implementation lint now rejects the shape (Rust, per the
#4439/#4447 implementation-language rule):

- `autospec-core::lint::implementation` gained a `BINARY_COMMITTED` deterministic
  detector. It fires on a committed binary file, or a mode-100755 file whose
  added lines carry no shebang (a compiled binary's signature). It is wired into
  `lint_implementation` and `commit_blocking_rules`, so `autospec lint
  implementation` and the pre-commit gate block the commit and emit a directive:
  remove the artefact, stage the files you changed by name (never `git add -A`),
  and add a `.gitignore` entry for the build output.
- The `DiffFile` model now captures the declared file mode from the diff header
  (`new file mode 100755` etc.), so the policy distinguishes an executable from
  a source file without inspecting the host checkout. The new detector lives in
  its own child module (`implementation/binary_committed.rs`) so the
  already-oversized `implementation.rs` is not grown (file-size ratchet).
- `AGENTS.md` (Engineering standards) records the staging-by-name rule and the
  reason.
- Tests in `crates/autospec-core/tests/implementation_lint_binary.rs`: a
  committed binary is rejected; an executable without a shebang is rejected; a
  shebang script and a plain source file are accepted; `BINARY_COMMITTED` is a
  commit-blocking rule; the directive names the `.gitignore` remedy.
