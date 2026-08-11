**Independent review adapter gate (mandatory before review dispatch):**

1. Resolve the commit-bound review risk using the canonical rules: Normal keeps one
   standard reviewer; High and Integration use high reasoning and prefer a provider
   different from the implementer; Critical requires a different provider. Integration
   and Critical require an examined integration path in the verdict.
2. Select a fresh, external, read-only foreground reviewer. Set foreground availability
   to `true` only when the harness can actually create and await that distinct context.
   The author/implementer context is never a reviewer fallback.
3. Run `independent-review-adapter.sh prepare` with the repository, issue, PR, exact
   40-hex head commit, risk, implementer provider, selected reviewer provider/reasoning,
   foreground availability, and a private request path. Exit `75` is a typed requeue:
   stop this issue immediately, do not invoke any merge command, and let the queue retry
   after an independent reviewer becomes available. Any other nonzero exit blocks.
4. Pass the generated JSON request plus `combined_reviewer_prompt` to exactly one
   foreground reviewer and capture exactly one JSON verdict artifact. Do not accept
   Markdown, prose, an author-context review, or a bare `LGTM` from the reviewer.
5. Run `independent-review-adapter.sh validate --request <request.json> --verdict
   <verdict.json>`. Only its exact `LGTM` output authorizes the existing outer gate.
   A blocked or malformed verdict remains non-authorizing and enters the repair loop.

The adapter path is `${AUTOSPEC_SCRIPTS_DIR:-$HOME/.autospec/scripts}/independent-review-adapter.sh`.
