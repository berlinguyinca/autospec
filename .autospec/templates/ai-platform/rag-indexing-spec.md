# RAG Indexing Spec

Define sources, chunking, embeddings, refresh cadence, citations, access control, and stale-index diagnostics.

## Purpose
Plan RAG indexing for target-repo knowledge.
## App-type applicability
Applies when docs, records, files, or domain data feed AI answers.
## Architecture recommendation
Separate ingestion, embedding, retrieval, citation, and permission checks.
## UI expectations
Show sources, freshness, citation links, and no-result states.
## Settings/config expectations
Configure sources, chunk size, embedding model, refresh, and exclusions.
## Tests required
Cover ingestion, retrieval, permission filtering, and citation rendering.
## Playwright expectations
Capture cited answer and no-context fallback flows.
## Docs/tutorial expectations
Document source onboarding and reindexing.
## Security/privacy notes
Enforce source permissions before retrieval.
## Acceptance criteria
- [ ] RAG source, embedding, citation, and permission behavior is specified.
## Validation commands
`bash scripts/autospec-ai-platform-audit.sh --dry-run`
## Metadata files expected to change
- `.autospec/state/ai-capabilities.json`
## Worker eligibility/risk notes
Specs and issue drafts are worker-eligible; data access needs guidance.
