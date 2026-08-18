#!/usr/bin/env bash

accountability_repo="${AUTOSPEC_FOREGROUND_ACCOUNTABILITY_REPO:-test/repo}"

if [ "$1" = api ] && printf '%s\n' "$@" | grep -q 'labels=autospec%3Arun-accountability'; then
  if [ -s "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY" ]; then
    jq -n --arg repo "$accountability_repo" --rawfile body "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY" '[[{"number":999,"url":("https://api.github.com/repos/"+$repo+"/issues/999"),"html_url":("https://github.com/"+$repo+"/issues/999"),"state":"open","body":$body,"labels":[{"name":"epic"},{"name":"type:tracker"},{"name":"no-auto"},{"name":"autospec:run-accountability"}]}]]'
  else
    printf '%s\n' '[[]]'
  fi
  exit 0
fi
if [ "$1" = label ] && [ "${2:-}" = create ]; then exit 0; fi
if [ "$1" = api ] && printf '%s\n' "$@" | grep -Fqx "repos/$accountability_repo/issues" && printf '%s\n' "$@" | grep -qx 'POST'; then
  payload="$(cat)"
  printf '%s' "$payload" | jq -r '.body' > "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY"
  jq -n --arg repo "$accountability_repo" --arg body "$(cat "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY")" '{number:999,url:("https://api.github.com/repos/"+$repo+"/issues/999"),html_url:("https://github.com/"+$repo+"/issues/999"),state:"open",body:$body,labels:[{name:"epic"},{name:"type:tracker"},{name:"no-auto"},{name:"autospec:run-accountability"}]}'
  exit 0
fi
if [ "$1" = issue ] && [ "${2:-}" = create ] && printf '%s\n' "$@" | grep -q 'Autonomous run'; then
  cat > "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY"
  printf 'https://github.com/%s/issues/999\n' "$accountability_repo"
  exit 0
fi
if [ "$1" = issue ] && [ "${2:-}" = edit ] && [ "${3:-}" = 999 ]; then
  cat > "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY"
  exit 0
fi
if [ "$1" = issue ] && [ "${2:-}" = view ] && [ "${3:-}" = 999 ]; then
  if [ -n "${AUTOSPEC_FOREGROUND_ACCOUNTABILITY_VIEW_COUNTER:-}" ]; then
    count=0
    if [ -s "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_VIEW_COUNTER" ]; then
      count="$(cat "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_VIEW_COUNTER")"
    fi
    count=$((count + 1))
    printf '%s\n' "$count" > "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY_VIEW_COUNTER"
    if [ "$count" -eq "${AUTOSPEC_FOREGROUND_ACCOUNTABILITY_TAMPER_AT_VIEW:-2}" ]; then
      jq -n --arg repo "$accountability_repo" '{number:999,url:("https://github.com/"+$repo+"/issues/999"),state:"OPEN",body:"tampered accountability body",labels:[{name:"epic"},{name:"type:tracker"},{name:"no-auto"},{name:"autospec:run-accountability"}]}'
      exit 0
    fi
  fi
  jq -n --arg repo "$accountability_repo" --rawfile body "$AUTOSPEC_FOREGROUND_ACCOUNTABILITY" '{number:999,url:("https://github.com/"+$repo+"/issues/999"),state:"OPEN",body:$body,labels:[{name:"epic"},{name:"type:tracker"},{name:"no-auto"},{name:"autospec:run-accountability"}]}'
  exit 0
fi
if [ "$1" = issue ] && [ "${2:-}" = close ] && [ "${3:-}" = 999 ]; then exit 0; fi
if [ "$1" = issue ] && [ "${2:-}" = reopen ] && [ "${3:-}" = 999 ]; then exit 0; fi
