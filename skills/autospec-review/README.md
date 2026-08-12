# autospec-review

Audits design specs against open + closed GitHub issues. Finds gaps,
writes them to `reports/autospec-review/gaps.csv`, and files
`[REGRESSION]` issues with `priority:high` + `regression` labels.

See `docs/specs/2026-05-06-autospec-review-design.md` in this repo for
full design.

## Usage

    /autospec-review [--spec PATH] [--profile NAME] [--dry-run]
                     [--no-autoreview] [--since DATE]
                     [--remediation] [--emit-gaps PATH] [--reasoning-trial]

See SKILL.md for full flag semantics and triggering rules.

`--reasoning-trial` is an opt-in remediation hardening pass for high-uncertainty
findings. Candidate gaps must carry a repo-local falsifier probe; the helper
records replayable JSONL events and emits only survivors for filing.

Remediation gaps remain backward compatible with the original required fields.
New gaps also carry either complete review attribution or the explicit status
`attribution_status: unavailable`; partial reviewer identity never validates as
an attributed clean sample.
