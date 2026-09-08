# Runbook — managed GitHub Project state recovery (automatic spec Projects)

Scope: recovery for the managed-Project state shipped in this revision —
the per-product store under `AUTOSPEC_HOME/projects/<product_key>/`
(`binding.json` + `events.jsonl`), the GitHub Projects v2 marker, and the
`autospec project resolve|sync|onboard|active-edges` CLI. Companion audit:
[`reports/autospec-review/automatic-spec-projects-phase55.md`](../../reports/autospec-review/automatic-spec-projects-phase55.md).

The governing principle is the same as for autonomous run-accountability
state: **the durable journal is authoritative; never repair GitHub and the
local journal by hand-editing either side.** Every command below is
read-only or fail-closed; the recovery action is always "rerun the same
command" unless this runbook says otherwise.

## 0. State layout and invariants

```
$AUTOSPEC_HOME/projects/<product_key>/
  binding.json    schema_version 2, identity, project_number/node_id/url,
                  owner, pending_projections, journal_digest,
                  journal_high_watermark
  events.jsonl    append-only journal; newest line is the high watermark
```

Invariants the store enforces on every open:

- The directory tree is private (`0700`/`0600`); a public or symlinked
  ancestor is refused (`managed project state directory permissions must be
  private`).
- A non-empty `binding.json` without a valid journal is refused, never
  repaired in place.
- Journal replay is read-only; duplicate event keys are no-ops; a torn
  newest line is dropped, not patched.
- The durable binding is written only *after* the GitHub Project identity
  and marker are verified (create → verify marker → persist).

## 1. Lost or ambiguous responses (create / marker edit)

**Symptoms.** A run exited non-zero with one of:

- `cannot create managed GitHub Project: <err>`
- `cannot write managed GitHub Project marker: <err>`
- `pending project creation has no verified project identity`
- `create_unknown: pending spec Project is not yet visible by its exact nonce title`

**Diagnose.**

```bash
STATE="$HOME/.autospec/projects/<product_key>"   # or $AUTOSPEC_HOME/projects/...
tail -5 "$STATE/events.jsonl"                    # look for the pending projection
grep -o '"project_number":[^,]*' "$STATE/binding.json" || echo "no binding yet"
gh project list --owner <owner> --format json    # see what actually exists
```

**Recover — do this, in this order:**

1. **Rerun the same command** (`autospec project resolve --repo-dir ...`).
   The CLI re-queries the owner's Projects, matches the *exact* managed
   marker (never title alone), resumes the journaled intent, and acks the
   pending projection only on a verified exact match. Lost responses resolve
   themselves on rerun; the marker edit is applied at most once.
2. If GitHub shows **exactly one** Project you recognize as the lost create:
   the rerun in step 1 will adopt it only if it carries the exact marker for
   this product. If it does not (create response lost *before* the marker
   write), the CLI reports `pending project creation has no verified project
   identity` — **do not hand-write the marker to make it match** unless you
   are certain the Project was created by autospec for this product (see
   §6, forbidden actions).
3. If GitHub shows **two or more** candidates, the CLI blocks with
   `multiple GitHub Projects have the managed marker for <product_key>` or
   a nonce-title count error. Resolve the ambiguity on the GitHub side
   (archive/delete the duplicate Project under GitHub's own controls) and
   rerun.

**Never:** delete `binding.json` or `events.jsonl` to "start clean" while a
pending create is journaled — that converts a recoverable ambiguity into an
unrecoverable double-create risk.

## 2. Duplicate or invalid managed markers

**Symptoms.**

- `GitHub Project managed marker must contain exactly one complete block`
- `managed GitHub Project marker owner <otherorg> conflicts with approved owner <owner>`
- `GitHub Project contains a different managed marker`
- `multiple GitHub Projects have the managed marker for <product_key>`

**Diagnose.** The marker is the fenced block in the Project README:

```
<!-- autospec-managed-project:begin -->
schema: 2
kind: product
product-key: <product_key>
owner: <owner>
<!-- autospec-managed-project:end -->
```

```bash
gh project view <number> --owner <owner> --format json | jq -r .readme
```

**Recover.**

- *Two blocks in one README:* edit the Project README to keep exactly one
  complete block (delete the stray one; keep human text outside the block).
  The CLI rewrites the block in place and never deletes human README text,
  so human edits outside the block survive reruns.
- *Owner mismatch / different-identity marker:* this is a hard identity
  conflict — the Project belongs to a different product/owner. **Do not
  overwrite the foreign marker.** Point the product at its own Project
  (fix `project_board.owner`/`product_key` in `.autospec/autonomous.yml`
  if those drifted) or create the correct Project and let the CLI bind it.
- *Two Projects with the same marker:* remove the duplicate on the GitHub
  side (archive/delete), then rerun `autospec project resolve`.

## 3. Identity drift (binding vs policy vs GitHub)

**Symptoms.**

- `managed project binding owner conflicts with policy`
- `verified remote project identity conflicts with local binding`
- `verified remote project identity conflicts with provisional creation`

**Meaning.** The durable binding, the approved policy, and the live GitHub
Project disagree on identity (owner, number, or node id). The CLI refuses
before any remote call when policy and binding disagree, and before any
mutation when GitHub and the binding disagree.

**Recover.**

1. Establish which side is correct by inspecting GitHub
   (`gh project view <number> --owner <owner> --format json`) and the policy
   (`.autospec/autonomous.yml` → `project_board`).
2. If **policy drifted** (e.g. org rename), fix the policy — the binding
   stays authoritative for what was already verified.
3. If **GitHub drifted** (Project deleted and re-created, or marker
   swapped), treat it per §1 (lost identity) or §2 (foreign marker). There
   is no supported in-place rebind; the state root for that product must be
   reset (§5) and re-resolved against the live Project.
4. If **both** changed (rename *and* re-create), reset (§5) and re-resolve.

## 4. Journal corruption

**Symptoms.**

- `invalid completed journal line: <err>` / `invalid journal event ...`
- store refuses to open the state root

**Recover.**

- A single torn newest line is dropped automatically on the next open
  (replay is read-only to the prefix; only the incomplete tail is lost).
  Rerun the command that was interrupted.
- For *any* other corruption, do **not** hand-edit `events.jsonl`. Reset the
  product state root per §5 and re-resolve from GitHub, which is the
  verified source of truth for Project identity once the marker is intact.
  If the marker itself is also gone, the rerun fails closed per §1/§2 and
  the ambiguity must be resolved on the GitHub side.

## 5. Safe state-root reset (last resort)

Only after §1–§4 have been exhausted and you have confirmed the intended
GitHub-side state:

```bash
# 1. Confirm the live GitHub side is unambiguous (one Project, exact marker).
gh project list --owner <owner> --format json
gh project view <number> --owner <owner> --format json | jq -r .readme

# 2. Reset the LOCAL projection state only.
rm -rf "$AUTOSPEC_HOME/projects/<product_key>"   # binding.json + events.jsonl

# 3. Re-resolve; adoption is by exact marker, so this rebinds to the same Project.
autospec project resolve --repo-dir <repo>
```

The reset discards local *projection* state (pending item additions,
tracked-issue URLs). Rerun `autospec project sync --repo-dir <repo>
--issue-url <url>` for each tracked issue to re-establish item membership;
reconcile is idempotent and will not duplicate items.

**Never reset while a create is still pending and its GitHub outcome is
unknown** — that is the double-create trap from §1.

## 6. Forbidden manual actions

- No hand-editing `binding.json` (digest-validated; edits silently break the
  journal digest and force a full reset).
- No hand-appending to `events.jsonl` (duplicate keys are no-ops, invented
  keys are refused).
- No writing or "fixing" the managed marker block by hand while a pending
  create/edit is journaled (use the CLI, or archive the Project and start
  clean).
- No running two autospec processes against the same `AUTOSPEC_HOME`
  concurrently: this revision ships **no lease/transaction surface**
  (`autospec project lease` and `autospec portfolio …` are refused as
  unknown commands). Concurrency is unsupported, not merely discouraged;
  divergence is detected on the next open but not arbitrated.
- No deleting the state root or individual files except via §5, and never
  via recursive deletes from a parent directory while a run may still be
  writing.

## 7. Completion and blockers

- **No command in this revision marks work done.** There is no
  `managed_state done` writer; repository-local completion stays **Blocked**
  until the deployment-owned consumer publishes a matching
  `autospec.implementation-handoff.v1` conformance receipt (audit §4).
  Do not fabricate a done state in the Project UI to unblock tooling —
  downstream gates read the receipt, not the board.
- **Blockers are not projected yet** (no blocker surface ships). Record
  blocking context on the GitHub issue/PR and in the Project README's
  human-written sections (outside the managed marker block); the completion
  gate simply cannot pass while the status surface is absent.

## 8. Quick diagnostic table

| Diagnostic (stderr) | Section | First action |
|---|---|---|
| `pending project creation has no verified project identity` | §1 | Rerun; then reconcile duplicates on GitHub |
| `cannot create managed GitHub Project: …` | §1 | Check `events.jsonl` pending create; inspect GitHub |
| `cannot write managed GitHub Project marker: …` | §1 | Rerun — resumes the journaled marker write |
| `cannot list GitHub Projects: …` | §1/§7 | Fix auth/scope (`project`), then rerun |
| `GitHub Project managed marker must contain exactly one complete block` | §2 | Remove the stray block from the README |
| `managed GitHub Project marker owner … conflicts with approved owner …` | §2 | Hard conflict — do not overwrite; fix policy or Project |
| `GitHub Project contains a different managed marker` | §2/§3 | Identity swap — verify, then §3 |
| `multiple GitHub Projects have the managed marker for …` | §2 | Archive the duplicate on GitHub, rerun |
| `managed project binding owner conflicts with policy` | §3 | Fix the drifted policy side |
| `verified remote project identity conflicts with …` | §3 | Verify GitHub side, reset + re-resolve if drifted |
| `managed project state directory permissions must be private` | §0 | `chmod 700` the state ancestors; never symlink state |
| `invalid completed journal line: …` | §4 | Rerun (torn tail auto-drops); else §5 reset |
| `unknown autospec project subcommand: lease` / `unknown autospec command: portfolio` | §6 | Expected — surface not shipped; do not work around it |
