# Companion Repository Map

AutoSpec uses companion repositories in three different ways. This page is the
canonical map for those relationships so operators can distinguish ordinary
target repos from catalog inputs and governance proposals.

## Target Repositories

Target repositories are the repos where AutoSpec plans, files, implements, and
validates work. The default single-repo path is `/autospec-define` followed by
`/autospec-run`; the multi-repo path is `autospec-fleet`.

`autospec-fleet` is a supervisor, not a replacement for `/autospec-run`:

- fleet owns clone/sync, repo-level scheduling, node-local capacity, and
  aggregate status;
- `/autospec-run` owns issue claiming, implementation, review, CI waiting, merge
  flow, and per-repo reporting;
- GitHub Issues and `autospec-run-state` comments remain the shared queue and
  lock layer across machines.

Reference: `docs/specs/2026-05-28-autospec-fleet-design.md`.

## Design Catalog

`autospec-design` fetches vendor design language files from the public
`berlinguyinca/awesome-design-md` catalog. It never vendors that catalog into
this repository.

Runtime access:

- primary: `gh api repos/berlinguyinca/awesome-design-md/...`;
- fallback: `curl` against `raw.githubusercontent.com`;
- cache: `~/.autospec/design-cache/<vendor>/DESIGN.md`;
- default cache TTL: 24 hours.

Reference: `skills/autospec-design/scripts/fetch-design-md.sh`.

## Companion Governance Proposals

The V65 companion sync bridge is proposal-only. It can generate inventories,
drift audits, compatibility checks, and manual patch bundles under
`.autospec/companions/v65`, but it must not write to companion repositories or
create companion PRs automatically.

Reference: `scripts/autospec-v61-v70.py` action group for V65.

## Current Gaps

- There is no hosted control plane for companion repositories.
- Fleet desired state can live in a Git control repository, but this repository
  ships only schemas, examples, and local validation.
- Companion governance is intentionally conservative until write bridges have
  separate proof and approval gates.
