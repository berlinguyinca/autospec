# Validator Fixture Reconciliation Design

**Issue:** #2072
**Date:** 2026-07-15
**Status:** approved by the existing Rust-cutover execution directive

## Problem

The direct Rust validator reports two false baseline failures on `origin/main`:
the dogfood allowlist retains the queue parser's old function name, and five
expanded skill documents no longer match their checked SHA-256 fixtures. Neither
failure represents a behavior change in the detector, expander, or runtime.

## Decision

Update only the stale fixture inputs. Replace `list_issues` with `read_issue` in
the one affected dogfood tuple, then regenerate the five stale skill goldens with
the repository's canonical generator. Do not change detector heuristics, skill
source, or validation ownership.

## Verification

`cargo run -q -p autospec-cli -- validate --fast` must pass. The dogfood driver
must report its expected queue tuple, and the block-expansion check must accept
the regenerated digests.
