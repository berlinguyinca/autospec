# AI Audit Log Spec

Record AI requests, tool calls, provider/model, token/cost summary, permission context, redacted inputs, and outcomes.

## Purpose
Plan audit logging for AI actions.
## App-type applicability
Applies to AI apps with user actions, tools, data access, or compliance needs.
## Architecture recommendation
Emit structured audit events at model call, tool call, and result boundaries.
## UI expectations
Expose audit summaries to authorized admins where useful.
## Settings/config expectations
Configure retention, redaction, event types, and export policy.
## Tests required
Cover event creation, redaction, and permission filtering.
## Playwright expectations
For admin UI, capture audit list/detail states.
## Docs/tutorial expectations
Document audit event meanings and retention.
## Security/privacy notes
Audit logs must redact prompts/secrets and follow retention policy.
## Acceptance criteria
- [ ] AI audit events and redaction policy are specified.
## Validation commands
`bash scripts/autospec-ai-platform-audit.sh --dry-run`
## Metadata files expected to change
- `.autospec/state/ai-capabilities.json`
## Worker eligibility/risk notes
Specs are worker-eligible; audit runtime changes need guidance.
