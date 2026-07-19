#!/usr/bin/env bash
set -euo pipefail

# --qa-gate)
# --qa-gate-pass-on-partial)
# scripts/explore-qa-gate.sh
# To merge sandbox into main
# Promotion WITHHELD
# sandbox QA: no QA config
# code_health:explore_qa_gate_failed
# sandbox_head_sha
# qa_gate_verdict
QA_GATE=0
if [ "$QA_GATE" -eq 0 ]; then
  true
fi
