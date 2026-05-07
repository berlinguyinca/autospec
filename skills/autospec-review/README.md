# autospec-review

Audits design specs against open + closed GitHub issues. Finds gaps,
writes them to `reports/autospec-review/gaps.csv`, and files
`[REGRESSION]` issues with `priority:high` + `regression` labels.

See `docs/specs/2026-05-06-autospec-review-design.md` in this repo for
full design.

## Usage

    /autospec-review [--spec PATH] [--profile NAME] [--dry-run]
                     [--no-autoreview] [--since DATE]

See SKILL.md for full flag semantics and triggering rules.
