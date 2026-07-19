# Ollama Provider Spec

Support local model endpoint configuration, availability diagnostics, model selection, and no-network fallback behavior.

## Purpose
Plan Ollama/local model support.
## App-type applicability
Applies to local-first or privacy-sensitive AI apps.
## Architecture recommendation
Use the same provider adapter as remote providers.
## UI expectations
Show local endpoint availability and model selection.
## Settings/config expectations
Configure endpoint, model, timeout, and no-context fallback.
## Tests required
Cover unavailable endpoint and model selection.
## Playwright expectations
Capture diagnostics and empty/error states for settings UI.
## Docs/tutorial expectations
Document local model setup and troubleshooting.
## Security/privacy notes
Do not assume local inference removes all privacy obligations.
## Acceptance criteria
- [ ] Local provider diagnostics and fallback are specified.
## Validation commands
`bash scripts/autospec-ai-platform-audit.sh --dry-run`
## Metadata files expected to change
- `.autospec/state/ai-capabilities.json`
## Worker eligibility/risk notes
Scaffold work is low risk; runtime integration requires bounded scope.
