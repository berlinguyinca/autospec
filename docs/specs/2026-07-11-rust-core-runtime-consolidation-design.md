# Rust Core Runtime Consolidation

## Validate Wrapper Fallback

`scripts/validate.sh` now attempts `autospec validate` before entering the
legacy shell body. The Rust command re-enters the shell with
`AUTOSPEC_FORCE_LEGACY_SHELL=1` while validation logic is still being ported.

The temporary fallback is tied to epic #1861. Remove it only after the remaining
runtime consolidation issues have moved the selected validation paths into Rust
and the shell wrapper is reduced to a thin compatibility entrypoint.
