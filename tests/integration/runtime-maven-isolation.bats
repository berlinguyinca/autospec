#!/usr/bin/env bats

setup() {
  export TEST_ROOT="$BATS_TEST_TMPDIR/runtime-maven-$BATS_TEST_NUMBER"
  export STATE_ROOT="$TEST_ROOT/state"
  export REPOSITORY="$TEST_ROOT/repository"
  export SETTINGS="$TEST_ROOT/settings.xml"
  export AUTOSPEC_BIN="$BATS_TEST_DIRNAME/../../target/debug/autospec"
  mkdir -p "$TEST_ROOT/a/consumer" "$TEST_ROOT/b/consumer" "$REPOSITORY"
  cp "$BATS_TEST_DIRNAME/../fixtures/runtime-resources/maven/producer/pom.xml" "$TEST_ROOT/a/pom.xml"
  cp "$BATS_TEST_DIRNAME/../fixtures/runtime-resources/maven/producer/pom.xml" "$TEST_ROOT/b/pom.xml"
  cp "$BATS_TEST_DIRNAME/../fixtures/runtime-resources/maven/consumer/pom.xml" "$TEST_ROOT/a/consumer/pom.xml"
  cp "$BATS_TEST_DIRNAME/../fixtures/runtime-resources/maven/consumer/pom.xml" "$TEST_ROOT/b/consumer/pom.xml"
  for environment in a b; do
    printf '%s\n' "version: 1" "default_mode: local" "modes:" "  local:" "    command: sh -c 'true'" > "$TEST_ROOT/$environment/.agent-runtime.yml"
  done
  printf 'artifact-a\n' > "$TEST_ROOT/a/payload.txt"
  printf 'artifact-b\n' > "$TEST_ROOT/b/payload.txt"
  printf '%s\n' '<settings><localRepository>'"$REPOSITORY"'</localRepository></settings>' > "$SETTINGS"
  export AGENT_ENV_STATE_ROOT="$STATE_ROOT"
  export MAVEN_ARGS="-s '$SETTINGS'"
}

@test "Maven 4 isolates same-GAV installs and shares downloaded artifacts" {
  run mvn --version
  [ "$status" -eq 0 ]
  [[ "$output" == *"Apache Maven 4."* ]]
  [ -x "$AUTOSPEC_BIN" ]

  for environment in a b; do
    run "$AUTOSPEC_BIN" runtime env exec --repo "$TEST_ROOT/$environment" -- \
      mvn -q org.apache.maven.plugins:maven-install-plugin:3.1.4:install-file \
      -Dfile=payload.txt -DgroupId=dev.autospec.collision -DartifactId=same-gav \
      -Dversion=1.0.0 -Dpackaging=txt -DgeneratePom=true
    [ "$status" -eq 0 ]
    run "$AUTOSPEC_BIN" runtime env exec --repo "$TEST_ROOT/$environment" -- \
      mvn -q -f consumer/pom.xml org.apache.maven.plugins:maven-dependency-plugin:3.8.1:copy-dependencies \
      -DoutputDirectory=resolved
    [ "$status" -eq 0 ]
  done

  run cmp "$TEST_ROOT/a/payload.txt" "$TEST_ROOT/a/consumer/resolved/same-gav-1.0.0.txt"
  [ "$status" -eq 0 ]
  run cmp "$TEST_ROOT/b/payload.txt" "$TEST_ROOT/b/consumer/resolved/same-gav-1.0.0.txt"
  [ "$status" -eq 0 ]
  run cmp "$TEST_ROOT/a/consumer/resolved/same-gav-1.0.0.txt" "$TEST_ROOT/b/consumer/resolved/same-gav-1.0.0.txt"
  [ "$status" -ne 0 ]

  [ "$(find "$REPOSITORY/autospec" -mindepth 1 -maxdepth 1 -type d | wc -l)" -eq 2 ]
  [ -d "$REPOSITORY/cached" ]
  [ "$(find "$REPOSITORY/cached" -type f | wc -l)" -gt 0 ]
}
