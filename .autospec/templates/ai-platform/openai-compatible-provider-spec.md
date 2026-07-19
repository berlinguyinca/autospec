# OpenAI-Compatible Provider Spec

Support API key references, base URL configuration, model listing assumptions, request/response normalization, and redacted logging.

## Purpose
Plan OpenAI-compatible provider support for a target repo.
## App-type applicability
Applies when remote or compatible local inference is in scope.
## Architecture recommendation
Use the provider adapter, not direct SDK calls spread through app code.
## UI expectations
Show provider status, selected model, and safe error messages.
## Settings/config expectations
Store API key references, base URL, model, timeout, and rate-limit settings.
## Tests required
Cover request mapping, response mapping, and redaction.
## Playwright expectations
For admin UI, test settings save, validation errors, and responsive layout.
## Docs/tutorial expectations
Document setup without raw secret values.
## Security/privacy notes
Never print API keys or Authorization headers.
## Acceptance criteria
- [ ] OpenAI-compatible provider behavior is traceable to config and tests.
## Validation commands
`bash scripts/autospec-ai-platform-audit.sh --dry-run`
## Metadata files expected to change
- `.autospec/state/ai-capabilities.json`
## Worker eligibility/risk notes
Docs/specs are low risk; credential/runtime handling needs guidance.
