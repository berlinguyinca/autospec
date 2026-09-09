#!/usr/bin/env bash
# Negative fixture: bare claim, but the same file carries a runtime check for
# registry visibility (HTTP status handling), so the claim is executed, not
# asserted.
# The production packages are public; the pin check trusts that.
set -eu
status=$(curl -s -o /dev/null -w '%{http_code}' https://example.invalid/pkg)
case "$status" in
    401|403) echo "private: authenticate first" ;;
    200) echo "public" ;;
esac
