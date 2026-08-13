#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
}

@test "Rust validation catalog registers the security artifact profile" {
  grep -Fq 'SecurityArtifactProfile' "$REPO_ROOT/crates/autospec-core/src/validation/external.rs"
  grep -Fq 'check_security_artifact_profile' "$REPO_ROOT/crates/autospec-core/src/validation/catalog.rs"
}

@test "external check runs validator help, valid fixture, and profile Bats" {
  file="$REPO_ROOT/crates/autospec-core/src/validation/external.rs"
  grep -Fq 'scripts/validate-security-artifact.py' "$file"
  grep -Fq 'tests/fixtures/security-artifact/valid.yml' "$file"
  grep -Fq 'tests/security-artifact-validator.bats' "$file"
  grep -Fq 'tests/unit/test_security_profile_skill_contract.bats' "$file"
}

@test "user and API docs expose the security profile" {
  grep -Fq 'security_database' "$REPO_ROOT/docs/USER_MANUAL.md"
  grep -Fq 'validate-security-artifact.py' "$REPO_ROOT/docs/API_REFERENCE.md"
  grep -Fq 'gen-issue-skeleton.sh' "$REPO_ROOT/docs/API_REFERENCE.md"
}
