#!/usr/bin/env bash

IMPLEMENTER_CONTRACT="implementer-contract.md"
REVIEWER_CONTRACT="reviewer-contract.md"

if [[ "${2:-}" == "implementer" ]]; then
  # Mirror the real bundler's framing, not just its contract text:
  # check_implementer_contract asserts that saved memory is the ONLY section below
  # the closing marker, and a stub with no markers at all cannot exercise that.
  echo "<!-- CACHE BOUNDARY -->"
  echo "implementer contract"
  echo "## Lockstep rules"
  echo "## Output discipline"
  echo "## Implementer scaffolding"
  echo "<!-- CACHE BOUNDARY -->"
  echo "## Project rules (saved memory)"
else
  echo "### RULE_ID table"
  echo "HALLUCINATED_API"
  echo "DUPLICATE_CODE"
  echo "STRING_MATCH_DOMAIN_LOGIC"
  echo "REPEATED_STRUCTURE_AS_CODE"
  echo "DOC_OUT_OF_SYNC"
  echo "INVENTED_CONFIG"
fi
