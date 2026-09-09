#!/usr/bin/env bash
# Positive fixture: bare host-capability claim, no tool probing in file.
# The worker has no container runtime, so the image step is skipped.
set -eu
echo skip
