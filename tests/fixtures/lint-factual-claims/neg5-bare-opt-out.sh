#!/usr/bin/env bash
# Negative fixture: the opt-out marker is bare (no reason), so it is rejected
# and the claim below stays a finding.
# linter:allow-FACTUAL_CLAIM
# The production packages are public.
set -eu
echo done
