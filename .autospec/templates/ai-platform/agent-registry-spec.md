# Agent Registry Spec

Define agent IDs, capabilities, tools, prompts, model policy, risk level, and validation evidence.

## Purpose
Plan a registry for AI agents.
## App-type applicability
Applies to apps with multiple AI workflows or personas.
## Architecture recommendation
Use a registry with explicit capabilities, tools, prompts, model policy, and risk.
## UI expectations
Expose agent purpose and safe status where administrators configure agents.
## Settings/config expectations
Track agent ID, owner, model policy, enabled state, and permissions.
## Tests required
Validate registry loading and disabled-agent behavior.
## Playwright expectations
For admin UI, test list/detail/error states.
## Docs/tutorial expectations
Document how agents are added and validated.
## Security/privacy notes
Agents must inherit permission and audit policies.
## Acceptance criteria
- [ ] Agent registry schema and validation expectations are documented.
## Validation commands
`bash scripts/autospec-ai-platform-audit.sh --dry-run`
## Metadata files expected to change
- `.autospec/state/ai-capabilities.json`
## Worker eligibility/risk notes
Registry specs are low risk; live agent behavior needs guidance.
