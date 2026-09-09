#!/usr/bin/env bash
# Fixture: process-matcher violation (pkill -f).
pkill -f "autospec worker" || true
