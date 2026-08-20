#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

# #3262 turned /autospec into a router, so it no longer carries any of this contract:
# the planning prose moved wholly to /autospec-define. Requiring it in both skills
# would force that refactor to be undone, so each case now checks the skill that owns
# the contract and, separately, that the router still reaches it.
@test "the planning skill defines the security database artifact contract" {
  # Pin the refactor's intent in both directions: the contract lives in exactly one
  # skill. If it reappears in the router, this fails and the narrowing above should be
  # revisited rather than silently covering two owners again.
  grep -Fq 'feature_profile: security_database' "$REPO_ROOT/skills/autospec/SKILL.md" \
    && { echo "the router carries the contract again; re-widen these loops"; return 1; }
  for skill in autospec-define; do
    file="$REPO_ROOT/skills/$skill/SKILL.md"
    grep -Fq 'feature_profile: security_database' "$file"
    grep -Fq '.autospec/spec-artifacts/<slug>.security-database.yml' "$file"
    grep -Fq 'validate-security-artifact.py' "$file"
    grep -Fq 'gen-issue-skeleton.sh' "$file"
    grep -Fq 'autospec:blocked-prerequisite' "$file"
    grep -Fq 'AUTHORITATIVE_CONTROL_MISSING' "$file"
    grep -Fq 'non-empty `gates` list' "$file"
    grep -Fq "one issue's \`produces\` list" "$file"
    grep -Fq 'ordinary profile' "$file"
  done
}

@test "security portfolio validation precedes issue creation" {
  for skill in autospec-define; do
    file="$REPO_ROOT/skills/$skill/SKILL.md"
    validation_line="$(grep -nF 'Portfolio validation gate' "$file" | head -n1 | cut -d: -f1)"
    creation_line="$(grep -nF 'Then create exactly N issues' "$file" | head -n1 | cut -d: -f1)"
    [ -n "$validation_line" ]
    [ -n "$creation_line" ]
    [ "$validation_line" -lt "$creation_line" ]
  done
}

@test "security profile contract is lock-step across harnesses" {
  for skill in autospec-define; do
    "$REPO_ROOT/scripts/derive-trio.sh" "$REPO_ROOT/skills/$skill" --check
    for file in \
      "$REPO_ROOT/skills/$skill/SKILL.md" \
      "$REPO_ROOT/skills/$skill/codex/prompt.md" \
      "$REPO_ROOT/skills/$skill/opencode/agent.md"
    do
      grep -Fq 'feature_profile: security_database' "$file"
      grep -Fq 'validate-security-artifact.py' "$file"
      grep -Fq 'gen-issue-skeleton.sh' "$file"
    done
  done
}

# Narrowing the loops above to the owning skill is only honest if something still
# proves the router reaches it. Without this, /autospec could stop delegating and
# every case above would keep passing.
@test "the autospec router reaches the skill that owns the security contract" {
  for member in SKILL.md codex/prompt.md opencode/agent.md; do
    file="$REPO_ROOT/skills/autospec/$member"
    grep -Fq 'skills/autospec-define/SKILL.md' "$file" \
      || { echo "skills/autospec/$member no longer delegates to autospec-define"; return 1; }
  done
}
