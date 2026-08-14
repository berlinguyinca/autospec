#!/usr/bin/env bash

if [ "$1" = api ] && printf '%s\n' "$@" | grep -q 'labels=autospec%3Arun-accountability'; then
  if [ -s "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY" ]; then
    jq -n --rawfile body "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY" '[[{"number":999,"url":"https://api.github.com/repos/test/repo/issues/999","html_url":"https://github.com/test/repo/issues/999","state":"open","body":$body,"labels":[{"name":"epic"},{"name":"type:tracker"},{"name":"no-auto"},{"name":"autospec:run-accountability"}]}]]'
  else
    printf '%s\n' '[[]]'
  fi
  exit 0
fi
if [ "$1" = label ] && [ "${2:-}" = create ]; then exit 0; fi
if [ "$1" = issue ] && [ "${2:-}" = create ] && printf '%s\n' "$@" | grep -q 'Autonomous run'; then
  cat > "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY"
  printf '%s\n' 'https://github.com/test/repo/issues/999'
  exit 0
fi
if [ "$1" = issue ] && [ "${2:-}" = edit ] && [ "${3:-}" = 999 ]; then
  cat > "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY"
  exit 0
fi
if [ "$1" = issue ] && [ "${2:-}" = view ] && [ "${3:-}" = 999 ]; then
  jq -n --rawfile body "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY" '{number:999,url:"https://github.com/test/repo/issues/999",state:"OPEN",body:$body,labels:[{name:"epic"},{name:"type:tracker"},{name:"no-auto"},{name:"autospec:run-accountability"}]}'
  exit 0
fi
