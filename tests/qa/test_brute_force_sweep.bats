#!/usr/bin/env bats
# tests/qa/test_brute_force_sweep.bats
#
# Fixture-driven test for the autospec-qa brute-force string-heuristics sweep
# introduced in issue #637. We synthesize offender files in each of the six
# supported languages (Python, JS/TS, Go, Java, Scala, Rust), mock `gh`, and
# assert that the sweep:
#   1. Emits a `code_health:brute_force_string_heuristics` finding per
#      language into `.autospec/qa-verdict.json`.
#   2. Files one `auto-implement,autospec:v2-flow` GitHub issue per offender
#      via `gh issue create`, with the file/function/line and the verbatim
#      RULE_ID directive in the body.
#
# Because the sweep itself is what's under test, mocking `gh` is the right
# call (per the issue scope). All other fixtures are real source files on
# disk that the sweep grep/heuristic pass will see.

REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
SWEEP="${REPO_ROOT}/scripts/qa-brute-force-sweep.sh"

setup() {
    TMPDIR_FIXT="$(mktemp -d)"
    export TMPDIR_FIXT
    mkdir -p "$TMPDIR_FIXT/.autospec"

    # synthetic offender — Python with rdkit imported + substring-on-name ladder
    mkdir -p "$TMPDIR_FIXT/src"
    cat >"$TMPDIR_FIXT/src/classify.py" <<'PY'
from rdkit import Chem

def classify(name):
    if "acid" in name:
        return ("acid", 1)
    if "alcohol" in name:
        return ("alcohol", 2)
    if "amine" in name:
        return ("amine", 3)
    if "ester" in name:
        return ("ester", 4)
    if "ether" in name:
        return ("ether", 5)
    if "ketone" in name:
        return ("ketone", 6)
    return ("unknown", 0)
PY

    # JS/TS offender with URL imported
    cat >"$TMPDIR_FIXT/src/route.ts" <<'TS'
const u = new URL(input);
function route(name: string) {
    if (name.includes("http")) return ["http", 1];
    if (name.includes("ftp")) return ["ftp", 2];
    if (name.includes("ssh")) return ["ssh", 3];
    if (name.includes("git")) return ["git", 4];
    if (name.includes("ws")) return ["ws", 5];
    return ["unknown", 0];
}
TS

    # Go offender with net/url import
    cat >"$TMPDIR_FIXT/src/classify.go" <<'GO'
package main
import "net/url"
var _ = url.Parse
func classify(s string) (string, int) {
    switch {
    case contains(s, "acid"): return "acid", 1
    case contains(s, "base"): return "base", 2
    case contains(s, "salt"): return "salt", 3
    case contains(s, "ion"):  return "ion", 4
    case contains(s, "gas"):  return "gas", 5
    }
    return "unknown", 0
}
GO

    # Java offender with JavaParser imported
    cat >"$TMPDIR_FIXT/src/Classify.java" <<'JAVA'
import com.github.javaparser.JavaParser;
public class Classify {
    public Object classify(String name) {
        if (name.contains("foo")) return new Object[]{"foo", 1};
        if (name.contains("bar")) return new Object[]{"bar", 2};
        if (name.contains("baz")) return new Object[]{"baz", 3};
        if (name.contains("qux")) return new Object[]{"qux", 4};
        if (name.contains("zap")) return new Object[]{"zap", 5};
        return new Object[]{"unknown", 0};
    }
}
JAVA

    # Scala offender with scalameta imported
    cat >"$TMPDIR_FIXT/src/Classify.scala" <<'SCALA'
import scala.meta._
object Classify {
  def classify(name: String): (String, Int) = name match {
    case n if n.contains("a") => ("a", 1)
    case n if n.contains("b") => ("b", 2)
    case n if n.contains("c") => ("c", 3)
    case n if n.contains("d") => ("d", 4)
    case n if n.contains("e") => ("e", 5)
    case _ => ("unknown", 0)
  }
}
SCALA

    # Rust offender with url::Url imported
    cat >"$TMPDIR_FIXT/src/classify.rs" <<'RUST'
use url::Url;
fn classify(name: &str) -> (&str, i32) {
    if name.contains("alpha") { return ("alpha", 1); }
    if name.contains("beta")  { return ("beta",  2); }
    if name.contains("gamma") { return ("gamma", 3); }
    if name.contains("delta") { return ("delta", 4); }
    if name.contains("eps")   { return ("eps",   5); }
    ("unknown", 0)
}
RUST

    # Mock gh — record invocations into a log file
    MOCK_BIN="$TMPDIR_FIXT/bin"
    mkdir -p "$MOCK_BIN"
    cat >"$MOCK_BIN/gh" <<'SH'
#!/usr/bin/env bash
echo "$@" >> "$TMPDIR_FIXT/gh-calls.log"
# emulate issue-create returning a URL
if [ "$1" = "issue" ] && [ "$2" = "create" ]; then
    echo "https://example/issues/9999"
fi
exit 0
SH
    chmod +x "$MOCK_BIN/gh"
    export PATH="$MOCK_BIN:$PATH"
}

teardown() {
    rm -rf "$TMPDIR_FIXT"
}

@test "sweep script exists and is executable" {
    [ -x "$SWEEP" ]
}

@test "sweep emits per-language findings in qa-verdict.json" {
    run env REPO_DIR="$TMPDIR_FIXT" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]
    for lang in python javascript go java scala rust; do
        run grep -F "$lang" "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -eq 0 ]
    done
    run grep -F 'code_health:brute_force_string_heuristics' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
    [ "$status" -eq 0 ]
}

@test "sweep files an auto-implement issue per offender via gh" {
    run env REPO_DIR="$TMPDIR_FIXT" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_FIXT/gh-calls.log" ]
    # Six languages -> at least six issue-create invocations.
    n=$(grep -c '^issue create' "$TMPDIR_FIXT/gh-calls.log" || true)
    [ "$n" -ge 6 ]
    # Each issue must carry the auto-implement + autospec:v2-flow labels.
    run grep -F 'auto-implement,autospec:v2-flow' "$TMPDIR_FIXT/gh-calls.log"
    [ "$status" -eq 0 ]
}

@test "sweep issue bodies cite RULE_ID directive verbatim" {
    run env REPO_DIR="$TMPDIR_FIXT" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    # At least one of the two new RULE_IDs must appear in the gh calls.
    run grep -E 'STRING_MATCH_DOMAIN_LOGIC|REPEATED_STRUCTURE_AS_CODE' "$TMPDIR_FIXT/gh-calls.log"
    [ "$status" -eq 0 ]
}
