# Inline Code Span Handling

This document exercises inline-code-span awareness in scan-doc-scope. A live
autospec-doc-scope block is a raw HTML comment standing on its own; an example
shown inside backticks (inline code) is illustrative prose and must NOT be parsed
as a live claim.

## Real Declaration

This is a real prose declaration and must be honored.

<!-- autospec-doc-scope:
  src: ["scripts/real.sh"]
  reason: "live prose declaration"
-->

Body for the real section.

## Illustrative Inline Example (well-formed)

The syntax is `<!-- autospec-doc-scope: src: ["scripts/inline-well-formed.sh"] -->`
and must be ignored even though it parses cleanly.

## Illustrative Inline Example (malformed, in a table cell)

| Feature | Annotation | Status |
| --- | --- | --- |
| Doc scopes | `<!-- autospec-doc-scope: src: ["glob"] reason: ... mismatch_action: warn\|hard_fail -->` | working |

This malformed inline example must not throw — it is documentation, not a claim.

## Post Inline Declaration

After the inline examples, this prose declaration must be honored again.

<!-- autospec-doc-scope:
  src: ["scripts/post-inline.sh"]
-->

Body for the post-inline section.
