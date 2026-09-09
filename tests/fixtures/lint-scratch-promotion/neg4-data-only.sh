#!/usr/bin/env bash
# Scratch paths that only ever appear as DATA (outputs, move targets, cleanup)
# are not invocations — a file written to /tmp is not a tool being used.
set -eu
rm -f /tmp/gw-as-3977-22910007/out/report.sh
mv /tmp/gw-as-3977-22910007/out/report.sh /tmp/gw-as-3977-22910007/out/report.sh.bak
> /tmp/gw-as-3977-22910007/out/report.sh
rm -f /tmp/gw-as-3977-22910007/out/report.sh
