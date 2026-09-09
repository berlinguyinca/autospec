# changelog.d/ — changelog fragments

`CHANGELOG.md` is **generated, not hand-edited**. Instead of every agent
appending to the top of `CHANGELOG.md` (which guarantees a merge conflict the
moment two agents land in the same run), each agent writes **one new file here**,
named after its issue. A release step concatenates the fragments into
`CHANGELOG.md` and clears this directory.

## Why

Agents work on isolated branches and rebase onto the latest `main` before
merging. Two agents both editing the same region of `CHANGELOG.md` (the top of
the `## [Unreleased]` section) produce overlapping hunks that git cannot
auto-merge — a guaranteed conflict on every concurrent run. Two agents writing
two *different files* never conflict; their patches apply to each other cleanly.

## Convention

- One fragment per landed change, named `<issue-number>-<short-slug>.md`
  (e.g. `3743-changelog-fragments.md`). The filename is what keeps concurrent
  fragments from colliding — never reuse or hand-edit another issue's fragment.
- A fragment is a **self-contained** block that reads correctly when placed at
  the top of the `## [Unreleased]` section. By convention it starts with a
  [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) category heading,
  followed by one or more bullets:

  ```markdown
  ### Added

  - Changelog is now generated from `changelog.d/` fragments; agents write a
    per-issue fragment instead of editing `CHANGELOG.md` (#3743, 2026-09-14).
  ```

- Do **not** edit `CHANGELOG.md` directly. Do **not** commit a fragment and then
  also edit `CHANGELOG.md` for the same change — the fragment *is* the entry.

## Release step

At release time, fold all fragments into `CHANGELOG.md` and clear this
directory:

```bash
bash scripts/build-changelog.sh            # fold + clear
bash scripts/build-changelog.sh --dry-run   # preview the [Unreleased] section only
```

Fragments are folded in filename (issue-number) order at the top of
`## [Unreleased]`, then removed. This directory and its `README.md` are the only
things that stay; an empty `changelog.d/` is the normal state between releases.
