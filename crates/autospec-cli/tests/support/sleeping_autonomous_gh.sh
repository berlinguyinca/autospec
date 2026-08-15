#!/bin/sh
set -eu

# A launched conductor must remain alive for lifecycle tests, but the parent start/restart
# command now has to create its mandatory accountability epic before it can detach.
fixture_mode=${AUTOSPEC_TEST_AUTONOMOUS_GH_MODE:-sleep}
if [ "${AUTOSPEC_ACCOUNTABILITY_REQUIRED:-}" = 1 ]; then
  if [ "$fixture_mode" = crash ]; then
    kill -KILL "$PPID"
    exit 1
  fi
  while kill -0 "$PPID" 2>/dev/null; do sleep 0.1; done
  exit 1
fi

if [ "$fixture_mode" = missing-health ] &&
   [ "$*" = "api repos/berlinguyinca/autospec/branches/missing-health" ]; then
  exit 1
fi

repo_from_args() {
  previous=
  for argument in "$@"; do
    if [ "$previous" = "--repo" ]; then
      printf '%s\n' "$argument"
      return 0
    fi
    case "$argument" in
      repos/*/issues*)
        repository=${argument#repos/}
        printf '%s\n' "${repository%%/issues*}"
        return 0
        ;;
    esac
    previous=$argument
  done
  return 1
}

repository=$(repo_from_args "$@" || printf '%s\n' test/repo)
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
slug=$(printf '%s' "$repository" | tr '/:' '__')
body_file="$script_dir/accountability-$slug.md"

if [ "$1" = api ] && printf '%s\n' "$@" | grep -q 'labels=autospec%3Arun-accountability'; then
  if [ -s "$body_file" ]; then
    jq -n --arg repo "$repository" --rawfile body "$body_file" '[[{"number":999,"url":("https://api.github.com/repos/"+$repo+"/issues/999"),"html_url":("https://github.com/"+$repo+"/issues/999"),"state":"open","body":$body,"labels":[{"name":"epic"},{"name":"type:tracker"},{"name":"no-auto"},{"name":"autospec:run-accountability"}]}]]'
  else
    printf '%s\n' '[[]]'
  fi
  exit 0
fi

if [ "$1" = label ] && [ "${2:-}" = create ]; then exit 0; fi

if [ "$1" = api ] && printf '%s\n' "$@" | grep -qx 'POST'; then
  temporary="$body_file.$$.tmp"
  jq -r '.body' > "$temporary"
  mv "$temporary" "$body_file"
  jq -n --arg repo "$repository" --rawfile body "$body_file" '{number:999,url:("https://api.github.com/repos/"+$repo+"/issues/999"),html_url:("https://github.com/"+$repo+"/issues/999"),state:"open",body:$body,labels:[{name:"epic"},{name:"type:tracker"},{name:"no-auto"},{name:"autospec:run-accountability"}]}'
  exit 0
fi

if [ "$1" = issue ] && [ "${2:-}" = edit ] && [ "${3:-}" = 999 ]; then
  temporary="$body_file.$$.tmp"
  cat > "$temporary"
  mv "$temporary" "$body_file"
  exit 0
fi

if [ "$1" = issue ] && [ "${2:-}" = view ] && [ "${3:-}" = 999 ]; then
  jq -n --arg repo "$repository" --rawfile body "$body_file" '{number:999,url:("https://github.com/"+$repo+"/issues/999"),state:"OPEN",body:$body,labels:[{name:"epic"},{name:"type:tracker"},{name:"no-auto"},{name:"autospec:run-accountability"}]}'
  exit 0
fi

if [ "$1" = issue ] && { [ "${2:-}" = close ] || [ "${2:-}" = reopen ]; } && [ "${3:-}" = 999 ]; then
  exit 0
fi

# Preserve the old fixture's long-running behavior for non-accountability GitHub work.
while kill -0 "$PPID" 2>/dev/null; do sleep 0.1; done
exit 1
