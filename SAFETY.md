# Safety

AutoSpec is designed for auditable agentic development, not blind delegation.

## Safety Principles

- Specs before implementation for non-trivial work.
- Small issues with explicit acceptance criteria.
- Worktree isolation for git-mutating workflows.
- Deterministic validation before success claims.
- Closeout reports that identify hidden failure risk.
- Human ownership of production, credentials, and destructive actions.

## Operator Responsibilities

- Run AutoSpec with the least credentials needed for the task.
- Review generated issues before high-risk implementation.
- Treat AI reviewer output as advisory until validation confirms it.
- Keep secrets out of prompts, specs, examples, and test fixtures.
- Stop or pause monitors when the scope is no longer correct.

## Not A Guarantee

AutoSpec reduces coordination risk, but it cannot guarantee that generated code is correct, secure, compliant, or appropriate for production. Use normal engineering review for security-sensitive, legal, financial, medical, infrastructure, and data-migration work.

