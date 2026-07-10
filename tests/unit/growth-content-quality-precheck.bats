setup() {
  SCRIPT="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts/growth-content-quality-precheck.sh"
  TMP="$(mktemp -d)"
}
teardown() { rm -rf "$TMP"; }

@test "clean prose passes" {
  printf 'AutoSpec ships specifications. It helps teams organize projects and manage changes. They review code before merging. Standards guide this process.\n' > "$TMP/c.md"
  run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -eq 0 ]
}
@test "keyword-stuffed content fails density" {
  # 'growth' 8 of ~10 words -> density 0.8 > 0.06
  printf 'growth growth growth growth growth growth growth growth tool now\n' > "$TMP/c.md"
  run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -ne 0 ]
  [[ "$output" == *density* ]]
}
@test "missing citation fails when GROWTH_MIN_CITATIONS=1" {
  printf 'The proposal outlines strategic objectives while discussing comprehensive methodology. Results from extensive testing appear quite promising. Implementation strategy involves multiple talented teams working collaboratively throughout different organizational phases. Milestones include careful planning activities and systematic execution steps during deployment.\n' > "$TMP/c.md"
  GROWTH_MIN_CITATIONS=1 run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -ne 0 ]
  [[ "$output" == *citation* ]]
}
@test "http link satisfies citation requirement" {
  printf 'The proposal outlines strategic objectives at https://example.com/bench while discussing comprehensive methodology. Results from extensive testing appear quite promising. Implementation strategy involves multiple talented teams working collaboratively throughout different phases.\n' > "$TMP/c.md"
  GROWTH_MIN_CITATIONS=1 run bash "$SCRIPT" "$TMP/c.md"; [ "$status" -eq 0 ]
}
@test "missing file rejected" { run bash "$SCRIPT" "$TMP/nope.md"; [ "$status" -ne 0 ]; }
