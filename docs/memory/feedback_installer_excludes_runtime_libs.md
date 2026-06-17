---
name: feedback_installer_excludes_runtime_libs
description: "install.sh excludes scripts/lib/ as \"install-time-only\" but it also holds runtime libs (autospec-loop.sh, autospec-harness-detect.sh) that 9 runtime scripts source at $SCRIPT_DIR/lib/ — clean install ships them nowhere, so autospec-explore hard-crashes on launch"
metadata: 
  node_type: memory
  type: feedback
  originSessionId: c279ed81-38f7-4098-ab47-a0a192f0366f
---

`scripts/lib/` is a MIXED directory: install-time-only helpers (`install-helpers.sh`,
`claude-md-block.txt`) **and** runtime libs (`autospec-loop.sh`,
`autospec-harness-detect.sh`). `install.sh copy_repo_scripts()` uses `find -maxdepth 1`
and explicitly excludes `scripts/lib/` ("install-time-only helpers"); the explicit
`runtime_skill_scripts` manifest (9 entries) omits the two runtime libs. Net: a clean
install never lands `~/.autospec/scripts/lib/autospec-loop.sh`, yet 9 runtime scripts
source it from `$SCRIPT_DIR/lib/` — `autospec-explore.sh:112/114` does so **un-guarded**
(`. "$SCRIPT_DIR/lib/..."`) and hard-crashes, while `autospec-continue.sh:52` wraps the
same source in `if [ -f ... ]` and degrades gracefully.

**Why:** the test suite is green while the install is broken — `validate.sh` only checks
scripts *reference* the lib and runs the loop's bats logic; `ship-completeness.bats` mirrors
the manifest that *omits* the libs. Classic [[feedback_self_consistent_test_fixtures_mask_bugs]]
blind spot: nothing asserts the installer actually copies runtime libs to the runtime tree.

**How to apply:** (1) add a `copy_runtime_libs()` step (or extend the manifest) that copies
`scripts/lib/autospec-loop.sh` + `autospec-harness-detect.sh` to `$AUTOSPEC_SCRIPTS_DIR/lib/`;
(2) add a ship-completeness assertion that every `$SCRIPT_DIR/lib/<x>` sourced by an installed
runtime script exists post-install; (3) make `autospec-explore.sh` guard its source like
`autospec-continue.sh` does. Immediate unblock: `cp scripts/lib/autospec-{loop,harness-detect}.sh
~/.autospec/scripts/lib/`. Related: [[feedback_skill_golden_derivation_workflow]].
