# A rule nobody asks you about is not a control (issue #4348)

Two cycles after filing #4312 ("a red platform job is the sole verification of
the code behind its `#[cfg]`"), the same author converted #4312: added a field
to `PersistedInvocation`, updated every initializer a Linux
`cargo build --workspace --all-targets` could see, and merged. One initializer
lives behind `#[cfg(any(target_os = "macos", target_os = "freebsd", windows))]`,
so the next run red-flagged `macos-test` and `freebsd-test` while `main-builds`
stayed green — precisely the shape the #4312 rule describes. The rule was
correct, recently written, and being actively worked on. **It still did not
fire**, because nothing in the workflow asks the question. "Documented" and
"enforced" are different states, and the gap is invisible until something slips
through it.

- **A rule that matters must become a step, a check, or a prompt — not a
  document entry.** The response to a self-violated rule is to add the check,
  not to resolve to remember harder: every "I knew that and did it anyway" is
  evidence about the system, not the person, and is spent on mechanising the
  rule rather than restating it.
- **When a change adds a field to a shared struct, enumerate its construction
  sites by text before trusting a build.** A compiler sees only this host's
  configuration; a text search (`grep 'Struct {'`) has no configuration.
  `invisible_to_host` is the difference between the text's view (all sites) and
  the build's view (the host's sites) — the sites where a missed
  field-initializer hides. The sites are supplied by the caller's text search,
  not read from the patch: the missed site's gate is not *in* the patch (the
  site is unchanged and far from any edited line), which is the whole reason
  the build cannot find it.
- **Where a `cfg`-gated module exists, there must be a supported way to check
  it locally.** The hold line names both remedies — the text search and the
  temporary-widen-and-build (`SiteVisibility::line`) — so the check is also the
  procedure.
- **The evaluator claims only what it can classify**, the way `platform_gate`
  does: a predicate classifies to a platform set only from platform terms and
  their `any`/`all`/`not` compositions; a term that is not a platform predicate
  (feature, `target_arch`, `test`) is platform-neutral and is skipped, and a
  composition with no platform term returns `None` (a pure feature/mode gate is
  not a platform restriction). Over-flagging is the safe direction — a needless
  hold, never a missed site.

Checkable in `autospec_core::construction_sites` (`ConstructionSite`,
`invisible_to_host`, `SiteVisibility`, `visibility`, `predicate_platforms`).
Tests: `crates/autospec-core/tests/construction_sites.rs`, including the
regression that reconstructs the incident (a `PersistedInvocation` with one
construction site gated to `macos`/`freebsd`/`windows` on a Linux host — the
check names that site, both remedies, and the `1 of 7` denominator) and the
control that on a host the site is gated to the check clears.
