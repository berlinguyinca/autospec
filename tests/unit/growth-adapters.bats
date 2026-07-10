setup() {
  DIR="$BATS_TEST_DIRNAME/../../skills/autospec-shared/scripts"
  TMP="$(mktemp -d)"
  export GROWTH_NOW_EPOCH=1000
  # Fake fetcher: ignore all args, print the fixture named by GROWTH_FIXTURE.
  cat > "$TMP/fetch.sh" <<'SH'
#!/usr/bin/env bash
cat "$GROWTH_FIXTURE"
SH
  chmod +x "$TMP/fetch.sh"
  export GROWTH_FETCH_CMD="$TMP/fetch.sh"
}
teardown() { rm -rf "$TMP"; }

@test "github adapter emits normalized github envelope" {
  echo '{"stargazers_count":10,"forks_count":3}' > "$TMP/gh.json"
  export GROWTH_FIXTURE="$TMP/gh.json" GITHUB_TOKEN=x
  echo '{"measurement":{"github":{"repo":"a/b","token_env":"GITHUB_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-github.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.provider')" = "github" ]
  [ "$(echo "$output" | jq -r '.metrics.stars')" = "10" ]
}
@test "github adapter fails closed when token env unset" {
  echo '{"measurement":{"github":{"repo":"a/b","token_env":"MISSING_TOK"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-github.sh" "$TMP/cfg.json"; [ "$status" -ne 0 ]
  [ -z "$output" ]
}
@test "analytics adapter emits normalized envelope" {
  echo '{"results":{"visitors":{"value":9},"pageviews":{"value":20}}}' > "$TMP/a.json"
  export GROWTH_FIXTURE="$TMP/a.json" PLAUSIBLE_API_TOKEN=x
  echo '{"measurement":{"analytics":{"provider":"plausible","site":"x.com","token_env":"PLAUSIBLE_API_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-analytics.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.metrics.visitors')" = "9" ]
}
@test "gsc adapter emits normalized envelope" {
  echo '{"rows":[{"keys":["q"],"clicks":5,"impressions":50,"position":7}]}' > "$TMP/g.json"
  export GROWTH_FIXTURE="$TMP/g.json" GSC_TOKEN=x
  echo '{"measurement":{"gsc":{"site":"x.com","token_env":"GSC_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-gsc.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.metrics.clicks_total')" = "5" ]
}
@test "rank adapter emits normalized envelope" {
  echo '{"keywords":[{"keyword":"a","position":4}]}' > "$TMP/r.json"
  export GROWTH_FIXTURE="$TMP/r.json" RANK_TOKEN=x
  echo '{"measurement":{"rank":{"endpoint":"https://rank.example","token_env":"RANK_TOKEN"}}}' > "$TMP/cfg.json"
  run bash "$DIR/growth-adapter-rank.sh" "$TMP/cfg.json"; [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq -r '.provider')" = "rank" ]
}
