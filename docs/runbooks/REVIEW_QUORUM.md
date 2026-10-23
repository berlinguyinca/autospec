# Review Quorum

Review quorum combines required specialist findings with verifier status before promotion to human review.

```bash
bash scripts/autospec-review-quorum.sh --dry-run --pr <number>
bash scripts/autospec-council-report.sh --dry-run --pr <number>
bash scripts/autospec-promote-pr.sh --dry-run --pr <number>
```

Quorum is an internal gate, not human approval. Blocked security/privacy/verifier findings block promotion.

## Head-bound post-PR quorum (`autospec review-quorum`)

The feature-scoped quorum above has no notion of a commit. After a PR exists a verdict is a statement about one exact head
SHA of one review target, so the post-PR quorum is evaluated by `autospec_core::autonomous::review_quorum` against the
CURRENT head only. There is ONE policy, implemented twice (this Rust evaluator for merge gating, Herdr's
`/api/v1/graph/nodes/{pr}/review-quorum` for the workbench) and pinned by a shared vector file,
`crates/autospec-core/tests/fixtures/review-quorum-vectors.json`, loaded by this crate's tests and by a byte-identical copy
in herdr-engineering (`tests/control_plane/fixtures/`). Change a rule in the vectors first.

```bash
autospec review-quorum --input quorum.json [--json] [--write-report .autospec/reports/review-quorum.json]
# exit 0 satisfied, 1 insufficient (re-review needed), 3 blocked, 2 invalid input
```

`quorum.json`: `{"target": "pr:github.com/o/r#7", "head": "<full sha>", "policy": {"required", "pr_author" (required),
"excluded_identities", "eligible_reviewers", "reviewer_groups", "reviewer_logins", "implementer_provider",
"require_provider_diversity"}, "votes": [{"reviewer", "target", "verdict", "commit", "receipt_digest",
"receipt_binding", "provider"}]}`. Build each vote from a receipt that `validate_review_receipt` accepted.

Rules (fail closed):

- Blocks are processed first and never dropped. A current-head `blocked` blocks whoever filed it, unless `eligible_reviewers`
  is non-empty and the voter is not on it (Herdr always has such a list). A block with a malformed reviewer id, an unparseable
  commit or a prefix of the head fails closed (blocks); a block for another head is stale; for another target, ignored.
  Targets compare case-insensitively.
- Matching is ASCII-only (identical in Herdr): whitespace is the six ASCII blanks, case folding touches only A-Z, and
  targets/ids must be printable ASCII. A non-ASCII target is an error; a non-ASCII reviewer id is invalid (a non-ASCII
  `blocked` still blocks). No Unicode folding (Kelvin sign, dotted I, NBSP).
- An `lgtm` counts only for this target and EXACTLY this head, from a valid eligible identity that is not the author
  (`x[bot]` equals `x`; `reviewer_logins` maps a key to a login) nor an excluded worker, with a 64-hex `receipt_digest` and a
  `receipt_binding` that recomputes: `sha256("pr-review-receipt-v1\0" lower(target) "\0" commit "\0" reviewer "\0" digest)`.
- Verdicts, identities and digests are trimmed and lower-cased. `reviewer_groups` entries count once. One digest backing
  `lgtm` votes of two groups voids both (replay). A missing `pr_author` is an error, never a pass.
- A force-push back to an already reviewed SHA restores that SHA's verdicts: the SHA names the content.

Residual (honest): the binding shows a stored vote was not copied across target, commit or reviewer; it is not a signature,
so a colluding key can recompute it. Two keys of one person count twice unless listed in a `reviewer_groups` entry. Provider
diversity is a normalised string compare here (the harness-aware `providers_are_diverse` lives in the executor bridge).
A verdict is bound to the head, not the base.

### Closing the loop with Herdr (off by default)

Config (YAML only, chmod 600 because it holds the reviewer's own ingest key; the key travels to `curl` in a 0600 header file,
never argv):

```yaml
herdr:
  enabled: false          # true to publish / request
  url: https://herdr.example.test
  ingest_key: <this reviewer's own ingest key>
  curl: curl
  dispatch_command: /path/to/your/review-dispatch   # called as: <cmd> -- <pr-node> <head>
  dispatch_timeout_seconds: 900                     # a pending claim older than this is re-claimed
```

- Producer: `autospec review-quorum publish --config C --request R --verdict V --pr-node pr:github.com/o/r#7` after
  `independent-review-adapter.sh validate` printed LGTM (a `blocked` verdict is always publishable). It POSTs a `review:*`
  node and a `reviewed` edge bound to the verdict's commit, with the receipt digest and its binding. Deterministic event ids
  make a re-publish a Herdr `duplicate`.
- Consumer: `autospec review-quorum request --config C --pr-node pr:github.com/o/r#7 [--state-dir D]` (run it from the
  same cron/loop that polls the PR) asks Herdr for the quorum and, when `rereview_required` is true for the current head, runs
  `dispatch_command` exactly once per (PR, head) (locked marker per pair; a failed dispatch is retried). Point `dispatch_command`
  at the existing review dispatch (e.g. a wrapper over `independent-review-adapter.sh prepare`).
- Merge path (fail closed). `autospec review-quorum --input I --write-report R [--repo-root D] [--operator-config C]` writes
  `review-quorum.json` atomically with `evaluated_at` (unix seconds) and `votes_digest`; on ANY evaluation error it OVERWRITES
  the report with a non-pass one (`verdict: invalid_input`, `promotion_ready: false`, head/target kept when readable) and exits
  2, so an older pass cannot survive a failed re-evaluation. `scripts/autospec-promote-pr.sh` (one line replaced, shell count
  unchanged) always calls `autospec review-quorum verify-report --repo-root D --pr N [--repo O/R]` and records the verified
  head as `review_quorum_head` in `promotion-plan.json` / `promotion-result.json`.
  - **Who requires the quorum is NOT the PR.** `required` comes only from trusted sources: the operator's config
    (`~/.autospec/autospec.yml`, or `--operator-config`) and the DEFAULT branch's copy of `.autospec/autospec.yml`, fetched
    FRESH from the forge at gate time (`gh api repos/O/R/contents/.autospec/autospec.yml`, run outside the checkout, for the
    explicit `--repo`; never a local `refs/remotes/*`, which a worker with `.git` write access can move with `git update-ref`,
    nor `remote.origin.url`). A forge answer that is not a 404, a 404 for a repository that cannot itself be read, and a missing
    `--repo` all mean required. The checkout under
    review can only ADD a requirement, never remove one (a PR that deletes the block and the report is still blocked). A trusted
    source that exists but cannot be read or parsed means required. The YAML is read with a real parser: anything under
    `review_quorum:` other than an explicit `required: false` (or `review_quorum: false`) requires it (`"true"`, `yes`,
    `True`, inline maps, a missing `required`, duplicates, null), and so does an enabled `herdr:` block in the operator config.
    `verify-report` (label promotion) keeps the old reading for a repository with no trusted configuration at all; the MERGE
    path does not (see `merge-gate`).
  - **`merge-gate` (the merge path, T16 H3/H4).** `autospec review-quorum merge-gate --repo-root D --pr N --repo O/R
    [--operator-config C]` is `verify-report` in strict mode and is what `scripts/autospec-guarded-merge.sh` and the run-drain
    recovery merge call. It differs: (1) no trusted source stating a `review_quorum` policy means REQUIRED, and only an explicit
    `review_quorum: {required: false}` in a trusted config opts out; (2) a required quorum needs origin proof, an
    `attestation_key_file` or an enabled Herdr live check; (3) the live head is read once and printed as `merge_head` (only on
    success); the scripts merge with `gh pr merge --match-head-commit <merge_head>`, so a head pushed after verification cannot
    merge; (4) any failure removes `autospec:verified` and `autospec:needs-human-review`; (5) a missing `autospec` binary, or one
    without `merge-gate`, refuses (exit non-zero), with no legacy fallback. `autospec-promote-pr.sh` likewise has no
    legacy fallback, and `--confirm` removes the same labels when it blocks or when the verified head differs from the head
    recorded by the previous promotion. Labels are invalidated at the next gate or promotion run, not on push.
  - When required, the report must be present, a non-empty object, a regular file inside the repo (a symlink, or a symlinked
    directory that resolves outside, is refused), from this evaluator (`source: head-bound-review-quorum`), `verdict: pass`,
    `promotion_ready: true`, no blocking findings, `evaluated_at` no older than `max_report_age_seconds` (default 900,
    trusted config) and not in the future, with `head` equal to the PR's LIVE head and `target_id` equal to that PR
    (`gh pr view --json headRefOid,url`); `--head SHA` additionally requires the live head to still be SHA (use it in any later
    label/merge step with the recorded `review_quorum_head`). A `gh` failure blocks. A report that exists but is empty or
    malformed blocks even when not required; the old Python report keeps its meaning when not required.
  - **Attestation (optional, enforced once configured).** `review_quorum.attestation_key_file: /abs/path` in a TRUSTED config:
    an absolute, regular (not symlink), 0600, user-owned file of 32..4096 bytes outside the repository and every git worktree.
    `--write-report` then adds `hmac` = HMAC-SHA256(key, `pr-review-report-v1\0 target \0 head \0 verdict \0 evaluated_at
    \0 votes_digest`) and `verify-report` requires it (constant-time compare); a bad key file overwrites the report with a
    non-pass one. Without a key the origin is UNPROVEN (anyone who can write the file can forge it) and `verify-report` prints
    `"origin":"unattested"` plus a warning. The key must be readable by whoever writes AND verifies reports.
  - **Live Herdr check.** When the operator config has `herdr.enabled: true` (a 0600 file with the ingest key), `verify-report`
    re-asks Herdr at verify time (`--no-live` skips it for one run) and blocks unless it answers `satisfied` at the live head,
    so a `blocked` filed after the report was written is seen; any Herdr error, or an unusable operator Herdr config, blocks.
  - Exit codes: 0 ok, 1 not trusted (JSON with `reasons`, `warnings`, `origin`, `live`, `live_head`), 2 usage. If the `autospec`
    binary is missing (or too old to know `verify-report`) `autospec-promote-pr.sh` blocks.
  Run `--write-report` after the feature-scoped Python quorum, which writes the same file.
- Retraction: `publish --retract` (same flags) sends an `edge.remove` for the SAME verdict file's edge with the same key; Herdr
  honours a remove only from the edge's owner, so a mistaken `blocked` is cleared by the same reviewer at the same head and by
  nobody else. After a retraction that exact verdict file cannot be re-published (same ids); file a new verdict.
- `request` safety. The default state dir is the operator's (`~/.autospec/state/review-requests`, scratch in
  `~/.autospec/state/herdr-scratch`), never inside a repository; created 0700, and an existing one must be a real directory
  owned by us and not group/other-writable. The dedupe is serialised by an exclusive flock on `<state-dir>/.lock`; each marker
  per (PR, head) is written whole (0600 temp file + rename) with states `pending` (owner pid, start, token), `dispatched`,
  `failed` (`failures`, `next_try_at`), and carries an HMAC under a per-install random nonce (`<state-dir>/.nonce`, 0600).
  A marker that is a symlink, has the wrong owner or mode, fails to parse, or has a bad MAC is NOT trusted: it counts as a
  pending claim of unknown age (its mtime) until `dispatch_timeout_seconds` + 30 s pass. A live pending owner blocks
  (`request_in_progress`); a dead owner or an expired claim is re-claimed; a failed dispatch backs off exponentially (30 s,
  60 s, ... capped at 1 h; `request_backoff`). `dispatch_command` runs as `<cmd> -- <pr-node> <head>` in its own process group
  and is killed (whole group) when it exceeds `dispatch_timeout_seconds` (default 900, 1..86400); the claim is held until the
  child is dead. The PR node is validated (`pr:` + printable ASCII, no shell metacharacters, must contain `#`). Herdr
  statuses `head_unverified`, `unknown_head`, `unknown_author` are ALERTS: `request` prints `alert:true`, writes an `ALERT`
  line to stderr and exits 4, never "not needed".
- Config hardening. `herdr.url` is parsed with a URL parser and normalised: https, or http only for exactly `127.0.0.1`,
  `::1` or `localhost`; userinfo, query and fragment are refused. The key header file is created in a private 0700 temp
  directory under `--scratch-dir` that is blanked and removed on every path (success, curl failing to spawn, early error); a
  SIGKILL cannot run the cleanup, but leaves only a 0700 directory. The YAML reader accepts comments, `---`, and other
  top-level blocks (skipped unread); inside `herdr:` it rejects tabs, nesting, misalignment, duplicate and unknown keys, and a
  second `herdr:` block; lists, anchors and multi-line scalars are not supported.
- Receipts: `publish` requires the request and verdict files to be JSON objects, both commits full-length and equal, the
  verdict exactly `lgtm`/`blocked`, and a request that names a PR (`pr`/`pr_node`/`target`) to name `--pr-node`.

The Python feature-scoped `review-quorum` is unchanged and is a porting candidate (#4447).
