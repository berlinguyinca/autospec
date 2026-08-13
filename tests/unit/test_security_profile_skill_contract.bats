#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "planning skills define the security database artifact contract" {
  for skill in autospec autospec-define; do
    file="$REPO_ROOT/skills/$skill/SKILL.md"
    grep -Fq 'feature_profile: security_database' "$file"
    grep -Fq '.autospec/spec-artifacts/<slug>.security-database.yml' "$file"
    grep -Fq 'validate-security-artifact.py' "$file"
    grep -Fq 'gen-issue-skeleton.sh' "$file"
    grep -Fq 'autospec:blocked-prerequisite' "$file"
    grep -Fq 'AUTHORITATIVE_CONTROL_MISSING' "$file"
    grep -Fq 'ordinary profile' "$file"
  done
}

@test "security portfolio validation precedes issue creation" {
  for skill in autospec autospec-define; do
    file="$REPO_ROOT/skills/$skill/SKILL.md"
    validation_line="$(grep -nF 'Portfolio validation gate' "$file" | head -n1 | cut -d: -f1)"
    creation_line="$(grep -nF 'Then create exactly N issues' "$file" | head -n1 | cut -d: -f1)"
    [ -n "$validation_line" ]
    [ -n "$creation_line" ]
    [ "$validation_line" -lt "$creation_line" ]
  done
}

@test "security profile contract is lock-step across harnesses" {
  for skill in autospec autospec-define; do
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
