#!/usr/bin/env bats
# tests/qa/test_brute_force_sweep.bats
#
# Fixture-driven test for the autospec-qa brute-force string-heuristics sweep
# introduced in issue #637 and refined in issue #640 (per-function scope).
#
# The sweep now scopes REPEATED_STRUCTURE_AS_CODE branch-counting to
# individual function bodies and emits findings only when 5+ same-shape
# branches appear within ONE function. Scattering branches across multiple
# functions must NOT emit a finding.

REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
SWEEP="${REPO_ROOT}/scripts/qa-brute-force-sweep.sh"

setup() {
    TMPDIR_FIXT="$(mktemp -d)"
    export TMPDIR_FIXT
    mkdir -p "$TMPDIR_FIXT/.autospec"

    # ----- positive offenders (5+ same-shape branches in ONE function) -----

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

    # ----- negative cases (#640): branches scattered across functions -----
    # 5 unrelated `if` lines across 5 different functions in each language —
    # must produce ZERO findings for REPEATED_STRUCTURE_AS_CODE.

    mkdir -p "$TMPDIR_FIXT/neg"

    cat >"$TMPDIR_FIXT/neg/scattered.py" <<'PY'
def a(x):
    if x > 0:
        return 1
def b(x):
    if x > 0:
        return 2
def c(x):
    if x > 0:
        return 3
def d(x):
    if x > 0:
        return 4
def e(x):
    if x > 0:
        return 5
PY

    cat >"$TMPDIR_FIXT/neg/scattered.ts" <<'TS'
function a(x: number) { if (x > 0) return 1; }
function b(x: number) { if (x > 0) return 2; }
function c(x: number) { if (x > 0) return 3; }
function d(x: number) { if (x > 0) return 4; }
function e(x: number) { if (x > 0) return 5; }
TS

    cat >"$TMPDIR_FIXT/neg/scattered.go" <<'GO'
package main
func a(x int) int { if x > 0 { return 1 }; return 0 }
func b(x int) int { if x > 0 { return 2 }; return 0 }
func c(x int) int { if x > 0 { return 3 }; return 0 }
func d(x int) int { if x > 0 { return 4 }; return 0 }
func e(x int) int { if x > 0 { return 5 }; return 0 }
GO

    cat >"$TMPDIR_FIXT/neg/Scattered.java" <<'JAVA'
public class Scattered {
    public int a(int x) { if (x > 0) return 1; return 0; }
    public int b(int x) { if (x > 0) return 2; return 0; }
    public int c(int x) { if (x > 0) return 3; return 0; }
    public int d(int x) { if (x > 0) return 4; return 0; }
    public int e(int x) { if (x > 0) return 5; return 0; }
}
JAVA

    cat >"$TMPDIR_FIXT/neg/Scattered.scala" <<'SCALA'
object Scattered {
  def a(x: Int): Int = { if (x > 0) return 1; 0 }
  def b(x: Int): Int = { if (x > 0) return 2; 0 }
  def c(x: Int): Int = { if (x > 0) return 3; 0 }
  def d(x: Int): Int = { if (x > 0) return 4; 0 }
  def e(x: Int): Int = { if (x > 0) return 5; 0 }
}
SCALA

    cat >"$TMPDIR_FIXT/neg/scattered.rs" <<'RUST'
fn a(x: i32) -> i32 { if x > 0 { return 1; } 0 }
fn b(x: i32) -> i32 { if x > 0 { return 2; } 0 }
fn c(x: i32) -> i32 { if x > 0 { return 3; } 0 }
fn d(x: i32) -> i32 { if x > 0 { return 4; } 0 }
fn e(x: i32) -> i32 { if x > 0 { return 5; } 0 }
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
    run env REPO_DIR="$TMPDIR_FIXT/src" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]
    for lang in python javascript go java scala rust; do
        run grep -F "\"language\":\"$lang\"" "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -eq 0 ]
    done
    run grep -F 'code_health:brute_force_string_heuristics' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
    [ "$status" -eq 0 ]
}

@test "sweep files an auto-implement issue per offender via gh" {
    run env REPO_DIR="$TMPDIR_FIXT/src" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
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
    run env REPO_DIR="$TMPDIR_FIXT/src" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    # At least one of the two new RULE_IDs must appear in the gh calls.
    run grep -E 'STRING_MATCH_DOMAIN_LOGIC|REPEATED_STRUCTURE_AS_CODE' "$TMPDIR_FIXT/gh-calls.log"
    [ "$status" -eq 0 ]
}

# ---------- issue #640 per-function-scope regression tests ----------

@test "per-function: positive offender cites the offending function name (python)" {
    run env REPO_DIR="$TMPDIR_FIXT/src" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    # Finding for classify.py must name the `classify` function, not "<unknown>"
    # and not the file's first def (here there is only one, but the rule still
    # applies).
    run grep -E '"file":"[^"]*classify\.py".*"function":"classify"' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
    [ "$status" -eq 0 ]
}

@test "per-function: positive offender cites the first branch line inside the function (python)" {
    run env REPO_DIR="$TMPDIR_FIXT/src" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    # First `if "acid" in name:` is on line 4 of the fixture.
    run grep -E '"rule_id":"REPEATED_STRUCTURE_AS_CODE".*"file":"[^"]*classify\.py".*"line":4' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
    [ "$status" -eq 0 ]
}

@test "per-function: scattered branches across 5 functions emit ZERO findings (python)" {
    run env REPO_DIR="$TMPDIR_FIXT/neg" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    if [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]; then
        run grep -F 'scattered.py' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -ne 0 ]
    fi
}

@test "per-function: scattered branches across 5 functions emit ZERO findings (javascript)" {
    run env REPO_DIR="$TMPDIR_FIXT/neg" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    if [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]; then
        run grep -F 'scattered.ts' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -ne 0 ]
    fi
}

@test "per-function: scattered branches across 5 functions emit ZERO findings (go)" {
    run env REPO_DIR="$TMPDIR_FIXT/neg" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    if [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]; then
        run grep -F 'scattered.go' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -ne 0 ]
    fi
}

@test "per-function: scattered branches across 5 functions emit ZERO findings (java)" {
    run env REPO_DIR="$TMPDIR_FIXT/neg" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    if [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]; then
        run grep -F 'Scattered.java' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -ne 0 ]
    fi
}

@test "per-function: scattered branches across 5 functions emit ZERO findings (scala)" {
    run env REPO_DIR="$TMPDIR_FIXT/neg" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    if [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]; then
        run grep -F 'Scattered.scala' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -ne 0 ]
    fi
}

@test "per-function: scattered branches across 5 functions emit ZERO findings (rust)" {
    run env REPO_DIR="$TMPDIR_FIXT/neg" VERDICT_FILE="$TMPDIR_FIXT/.autospec/qa-verdict.json" \
        bash "$SWEEP"
    [ "$status" -eq 0 ]
    if [ -f "$TMPDIR_FIXT/.autospec/qa-verdict.json" ]; then
        run grep -F 'scattered.rs' "$TMPDIR_FIXT/.autospec/qa-verdict.json"
        [ "$status" -ne 0 ]
    fi
}
