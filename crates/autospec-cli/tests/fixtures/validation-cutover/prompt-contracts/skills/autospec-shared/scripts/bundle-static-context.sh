#!/usr/bin/env bash

IMPLEMENTER_CONTRACT="implementer-contract.md"
REVIEWER_CONTRACT="reviewer-contract.md"

if [[ "${2:-}" == "implementer" ]]; then
  echo "implementer contract"
else
  echo "### RULE_ID table"
  echo "HALLUCINATED_API"
  echo "DUPLICATE_CODE"
  echo "STRING_MATCH_DOMAIN_LOGIC"
  echo "REPEATED_STRUCTURE_AS_CODE"
  echo "DOC_OUT_OF_SYNC"
  echo "INVENTED_CONFIG"
fi
