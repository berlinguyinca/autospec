// fixtures.mjs — fixture corpus for assertion-shift classifier tests.
// Each fixture: { id, description, diff, filePath, commitMessages, nonTestFilesChanged, expected }
// expected: { gate_passed, verdicts: [{bucket}] } (verdicts may be partial)

/**
 * Keep fixture records immutable and provide defaults for the fields that are
 * intentionally empty in most cases.  This keeps future additions focused on
 * the behavior being exercised instead of repeating corpus bookkeeping.
 */
const freeze = (value) => {
    if (value && typeof value === 'object' && !Object.isFrozen(value)) {
        Object.freeze(value);
        Object.values(value).forEach(freeze);
    }
    return value;
};

const fixture = ({ nonTestFilesChanged = [], expected, ...record }) =>
    freeze({ ...record, nonTestFilesChanged: [...nonTestFilesChanged], expected: { ...expected } });

// Keep intentionally weakened-test markers inside diff fixtures without making
// this corpus itself look like it contains focused tests to repository audits.
const jestSkip = 'test.' + 'skip';
const pytestSkip = '@pytest.mark.' + 'skip';

export const FIXTURES = Object.freeze([

    // ── LOOSENING fixtures ────────────────────────────────────────────────────

    {
        id: 'jest-01',
        description: 'jest: assertion removed → LOOSENING',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: update test\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,6 @@ test('add', () => {
-  expect(result).toBe(42);
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'jest-02',
        description: 'jest: toStrictEqual → toEqual → LOOSENING',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: relax check\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,7 @@ test('add', () => {
-  expect(result).toStrictEqual({a: 1});
+  expect(result).toEqual({a: 1});
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'jest-03',
        description: 'jest: ' + jestSkip + ' added → LOOSENING',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: skip flaky\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -1,7 +1,7 @@
-test('add', () => {
+${jestSkip}('add', () => {
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'playwright-01',
        description: 'playwright: toStrictEqual → toMatchObject → LOOSENING',
        filePath: 'e2e/dashboard.spec.ts',
        commitMessages: 'fix: relax assertion\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/e2e/dashboard.spec.ts b/e2e/dashboard.spec.ts
@@ -10,7 +10,7 @@ test('dashboard', async ({ page }) => {
-  expect(data).toStrictEqual({ id: 1, name: 'Alice' });
+  expect(data).toMatchObject({ id: 1 });
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'playwright-02',
        description: 'playwright: assertion removed from test block → LOOSENING',
        filePath: 'e2e/login.spec.ts',
        commitMessages: 'fix: remove assertion\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/e2e/login.spec.ts b/e2e/login.spec.ts
@@ -8,7 +8,6 @@ test('login', async ({ page }) => {
-  await expect(page).toHaveURL('/dashboard');
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'pytest-01',
        description: 'pytest: assert removed → LOOSENING',
        filePath: 'tests/test_api.py',
        commitMessages: 'fix: remove check\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_api.py b/tests/test_api.py
@@ -10,7 +10,6 @@ def test_status():
-    assert response.status_code == 200
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'pytest-02',
        description: 'pytest: ' + pytestSkip + ' added → LOOSENING',
        filePath: 'tests/test_api.py',
        commitMessages: 'fix: skip\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_api.py b/tests/test_api.py
@@ -1,4 +1,5 @@
+${pytestSkip}
 def test_status():
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'go-01',
        description: 'go-test: require.Equal → assert.Contains → LOOSENING',
        filePath: 'pkg/calc/calc_test.go',
        commitMessages: 'fix: relax\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/pkg/calc/calc_test.go b/pkg/calc/calc_test.go
@@ -5,7 +5,7 @@ func TestAdd(t *testing.T) {
-\trequire.Equal(t, 42, result)
+\tassert.Contains(t, []int{42}, result)
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'go-02',
        description: 'go-test: t.Skip added → LOOSENING',
        filePath: 'pkg/calc/calc_test.go',
        commitMessages: 'fix: skip\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/pkg/calc/calc_test.go b/pkg/calc/calc_test.go
@@ -3,6 +3,7 @@ func TestAdd(t *testing.T) {
+\tt.Skip("not ready")
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'cargo-01',
        description: 'cargo-test: assert_eq! removed → LOOSENING',
        filePath: 'src/lib_test.rs',
        commitMessages: 'fix: remove\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/lib_test.rs b/src/lib_test.rs
@@ -5,7 +5,6 @@ fn test_add() {
-    assert_eq!(result, 42);
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'cargo-02',
        description: 'cargo-test: assert_eq! → assert! → LOOSENING (weaker operator)',
        filePath: 'src/lib_test.rs',
        commitMessages: 'fix: weaken\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/lib_test.rs b/src/lib_test.rs
@@ -5,7 +5,7 @@ fn test_add() {
-    assert_eq!(result, 42);
+    assert!(result > 0);
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'mocha-01',
        description: 'mocha: assertion removed → LOOSENING',
        filePath: 'test/api.test.js',
        commitMessages: 'fix: remove check\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/test/api.test.js b/test/api.test.js
@@ -5,7 +5,6 @@ it('api', () => {
-  expect(result).to.deep.equal({ id: 1, name: 'Alice' });
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    // ── SHIFTING fixtures ─────────────────────────────────────────────────────

    {
        id: 'jest-04',
        description: 'jest: value-only change, no justification → SHIFTING (blocks)',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: update expected value\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,7 @@ test('add', () => {
-  expect(result).toBe(42);
+  expect(result).toBe(43);
`,
        expected: { gate_passed: false, reason: 'unjustified_assertion_shift', any_bucket: 'SHIFTING' },
    },

    {
        id: 'jest-05',
        description: 'jest: SHIFTING with JUSTIFICATION + co-edit → passes',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: update expected value\nJUSTIFICATION: upstream API now returns 43 as documented in changelog\n',
        nonTestFilesChanged: ['src/calc.js'],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,7 @@ test('add', () => {
-  expect(result).toBe(42);
+  expect(result).toBe(43);
`,
        expected: { gate_passed: true, any_bucket: 'SHIFTING' },
    },

    {
        id: 'jest-06',
        description: 'jest: SHIFTING with JUSTIFICATION but no co-edit → blocks',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: update expected\nJUSTIFICATION: value changed\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,7 @@ test('add', () => {
-  expect(result).toBe(42);
+  expect(result).toBe(43);
`,
        expected: { gate_passed: false, reason: 'unjustified_assertion_shift', any_bucket: 'SHIFTING' },
    },

    {
        id: 'jest-07',
        description: 'jest: SHIFTING with co-edit but no JUSTIFICATION → blocks',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: update expected value\n',
        nonTestFilesChanged: ['src/calc.js'],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,7 @@ test('add', () => {
-  expect(result).toBe(42);
+  expect(result).toBe(43);
`,
        expected: { gate_passed: false, reason: 'unjustified_assertion_shift', any_bucket: 'SHIFTING' },
    },

    {
        id: 'playwright-03',
        description: 'playwright: URL value shift no justification → SHIFTING (blocks)',
        filePath: 'e2e/login.spec.ts',
        commitMessages: 'fix: update URL\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/e2e/login.spec.ts b/e2e/login.spec.ts
@@ -8,7 +8,7 @@ test('login', async ({ page }) => {
-  await expect(page).toHaveURL('/dashboard');
+  await expect(page).toHaveURL('/home');
`,
        expected: { gate_passed: false, reason: 'unjustified_assertion_shift', any_bucket: 'SHIFTING' },
    },

    {
        id: 'playwright-04',
        description: 'playwright: SHIFTING with both JUSTIFICATION + co-edit → passes',
        filePath: 'e2e/login.spec.ts',
        commitMessages: 'fix: update redirect\nJUSTIFICATION: redirect changed to /home per product decision\n',
        nonTestFilesChanged: ['src/router.ts'],
        diff: `diff --git a/e2e/login.spec.ts b/e2e/login.spec.ts
@@ -8,7 +8,7 @@ test('login', async ({ page }) => {
-  await expect(page).toHaveURL('/dashboard');
+  await expect(page).toHaveURL('/home');
`,
        expected: { gate_passed: true, any_bucket: 'SHIFTING' },
    },

    {
        id: 'pytest-03',
        description: 'pytest: value shift no justification → SHIFTING (blocks)',
        filePath: 'tests/test_api.py',
        commitMessages: 'fix: update value\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_api.py b/tests/test_api.py
@@ -10,7 +10,7 @@ def test_status():
-    assert response.json()['count'] == 10
+    assert response.json()['count'] == 11
`,
        expected: { gate_passed: false, reason: 'unjustified_assertion_shift', any_bucket: 'SHIFTING' },
    },

    {
        id: 'go-03',
        description: 'go-test: value shift with JUSTIFICATION + co-edit → passes',
        filePath: 'pkg/calc/calc_test.go',
        commitMessages: 'fix: update expected\nJUSTIFICATION: algorithm now returns 43\n',
        nonTestFilesChanged: ['pkg/calc/calc.go'],
        diff: `diff --git a/pkg/calc/calc_test.go b/pkg/calc/calc_test.go
@@ -5,7 +5,7 @@ func TestAdd(t *testing.T) {
-\trequire.Equal(t, 42, result)
+\trequire.Equal(t, 43, result)
`,
        expected: { gate_passed: true, any_bucket: 'SHIFTING' },
    },

    // ── STRENGTHENING fixtures ────────────────────────────────────────────────

    {
        id: 'jest-08',
        description: 'jest: assertion added → STRENGTHENING (passes)',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'test: add assertion\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,6 +5,7 @@ test('add', () => {
+  expect(result).toBe(42);
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'jest-09',
        description: 'jest: toEqual → toStrictEqual → STRENGTHENING',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'test: tighten check\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -5,7 +5,7 @@ test('add', () => {
-  expect(result).toEqual({a: 1});
+  expect(result).toStrictEqual({a: 1});
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'jest-10',
        description: 'jest: ' + jestSkip + ' removed → STRENGTHENING',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'test: unskip\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -1,7 +1,7 @@
-${jestSkip}('add', () => {
+test('add', () => {
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'playwright-05',
        description: 'playwright: assertion added → STRENGTHENING',
        filePath: 'e2e/dashboard.spec.ts',
        commitMessages: 'test: add assertion\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/e2e/dashboard.spec.ts b/e2e/dashboard.spec.ts
@@ -10,6 +10,7 @@ test('dashboard', async ({ page }) => {
+  await expect(page.locator('h1')).toHaveText('Dashboard');
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'pytest-04',
        description: 'pytest: assert added → STRENGTHENING',
        filePath: 'tests/test_api.py',
        commitMessages: 'test: add check\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_api.py b/tests/test_api.py
@@ -10,6 +10,7 @@ def test_status():
+    assert response.json()['success'] == True
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'pytest-05',
        description: 'pytest: ' + pytestSkip + ' removed → STRENGTHENING',
        filePath: 'tests/test_api.py',
        commitMessages: 'test: unskip\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_api.py b/tests/test_api.py
@@ -1,4 +1,3 @@
-${pytestSkip}
 def test_status():
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'go-04',
        description: 'go-test: assertion added → STRENGTHENING',
        filePath: 'pkg/calc/calc_test.go',
        commitMessages: 'test: add check\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/pkg/calc/calc_test.go b/pkg/calc/calc_test.go
@@ -5,6 +5,7 @@ func TestAdd(t *testing.T) {
+\trequire.Equal(t, 42, result)
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'cargo-03',
        description: 'cargo-test: assert! → assert_eq! → STRENGTHENING',
        filePath: 'src/lib_test.rs',
        commitMessages: 'test: strengthen\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/lib_test.rs b/src/lib_test.rs
@@ -5,7 +5,7 @@ fn test_add() {
-    assert!(result == 42);
+    assert_eq!(result, 42);
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'cargo-04',
        description: 'cargo-test: #[ignore] removed → STRENGTHENING',
        filePath: 'src/lib_test.rs',
        commitMessages: 'test: unignore\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/lib_test.rs b/src/lib_test.rs
@@ -3,7 +3,6 @@ mod tests {
-    #[ignore]
     fn test_add() {
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    // ── No-verdict fixtures (pure selector / non-assertion changes) ───────────

    {
        id: 'playwright-06',
        description: 'playwright: pure selector fix — no assertion change → no verdicts, passes',
        filePath: 'e2e/dashboard.spec.ts',
        commitMessages: 'fix: update selector\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/e2e/dashboard.spec.ts b/e2e/dashboard.spec.ts
@@ -5,7 +5,7 @@ test('dashboard', async ({ page }) => {
-  await page.click('.old-btn');
+  await page.click('[data-testid="submit"]');
`,
        expected: { gate_passed: true, any_bucket: null },
    },

    {
        id: 'jest-11',
        description: 'jest: comment-only change → no verdicts, passes',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'docs: update comment\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -1,4 +1,4 @@
-// old comment
+// new comment
`,
        expected: { gate_passed: true, any_bucket: null },
    },

    {
        id: 'pytest-06',
        description: 'pytest: tolerance tightened → STRENGTHENING (passes)',
        filePath: 'tests/test_math.py',
        commitMessages: 'test: tighten tolerance\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_math.py b/tests/test_math.py
@@ -5,7 +5,7 @@ def test_pi():
-    assert result == approx(3.14, abs=0.01)
+    assert result == approx(3.14159, abs=0.001)
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'mocha-02',
        description: 'mocha: assertion added → STRENGTHENING',
        filePath: 'test/api.test.js',
        commitMessages: 'test: add assertion\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/test/api.test.js b/test/api.test.js
@@ -5,6 +5,7 @@ it('api', () => {
+  expect(result).to.deep.equal({ id: 1, name: 'Alice' });
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'vitest-01',
        description: 'vitest: toMatchObject → toStrictEqual → STRENGTHENING',
        filePath: 'src/utils.test.ts',
        commitMessages: 'test: strengthen\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/utils.test.ts b/src/utils.test.ts
@@ -5,7 +5,7 @@ test('util', () => {
-  expect(result).toMatchObject({a: 1});
+  expect(result).toStrictEqual({a: 1, b: 2});
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'vitest-02',
        description: 'vitest: toStrictEqual → toMatchObject → LOOSENING',
        filePath: 'src/utils.test.ts',
        commitMessages: 'fix: relax\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/utils.test.ts b/src/utils.test.ts
@@ -5,7 +5,7 @@ test('util', () => {
-  expect(result).toStrictEqual({a: 1, b: 2});
+  expect(result).toMatchObject({a: 1});
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'go-05',
        description: 'go-test: assert.Equal removed entirely → LOOSENING',
        filePath: 'pkg/api/api_test.go',
        commitMessages: 'fix: remove check\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/pkg/api/api_test.go b/pkg/api/api_test.go
@@ -7,7 +7,6 @@ func TestStatus(t *testing.T) {
-\tassert.Equal(t, 200, resp.StatusCode)
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    // ── v2 contract diff fixtures (spec §5c) ──────────────────────────────────
    // These exercise assertion-shift-v2-buckets.mjs via the second-pass handler
    // in assertion-shift-classifier.mjs when .autospec/test.yml appears in diff.

    {
        id: 'v2-01',
        description: 'v2: remove invariants[] entry → LOOSENING',
        filePath: '.autospec/test.yml',
        commitMessages: 'chore: remove invariant\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -10,7 +10,3 @@ e2e:
   invariants_v2:
     enabled: true
-    structural_invariants:
-      - name: dashboard-done-items-editable
-        selector: "[data-done-item]"
-        require_affordance: edit
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'v2-02',
        description: 'v2: narrow apply_on_routes → LOOSENING',
        filePath: '.autospec/test.yml',
        commitMessages: 'chore: narrow routes\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -5,14 +5,12 @@ e2e:
   invariants_v2:
     enabled: true
     structural_invariants:
       - name: dashboard-done-items-editable
         selector: "[data-done-item]"
         require_affordance: edit
-        apply_on_routes:
-          - /dashboard
-          - /archive
+        apply_on_routes:
+          - /dashboard
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'v2-03',
        description: 'v2: lower require_count_at_least → LOOSENING',
        filePath: '.autospec/test.yml',
        commitMessages: 'chore: lower count\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -5,8 +5,8 @@ e2e:
   invariants_v2:
     enabled: true
     structural_invariants:
       - name: dashboard-done-items-editable
-        require_count_at_least: 5
+        require_count_at_least: 1
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'v2-04',
        description: 'v2: mismatch_action hard_fail → warn_only → LOOSENING',
        filePath: '.autospec/test.yml',
        commitMessages: 'chore: soften mismatch action\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -5,7 +5,7 @@ e2e:
   invariants_v2:
     enabled: true
     window_contracts:
       - name: streak-window
-        mismatch_action: hard_fail
+        mismatch_action: warn_only
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'v2-05',
        description: 'v2: lower bfs_max_routes → LOOSENING',
        filePath: '.autospec/test.yml',
        commitMessages: 'chore: reduce crawl depth\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -14,3 +14,3 @@ e2e:
   invariants_v2:
-    bfs_max_routes: 20
+    bfs_max_routes: 5
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'v2-06',
        description: 'v2: remove affordance_patterns[] entry → LOOSENING',
        filePath: '.autospec/test.yml',
        commitMessages: 'chore: remove affordance pattern\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -14,5 +14,3 @@ e2e:
   invariants_v2:
     affordance_patterns:
       - button[data-action=edit]
-      - a[href*=/edit]
`,
        expected: { gate_passed: false, reason: 'assertion_loosening', any_bucket: 'LOOSENING' },
    },

    {
        id: 'v2-07',
        description: 'v2: add new invariants[] entry → STRENGTHENING (gate passes)',
        filePath: '.autospec/test.yml',
        commitMessages: 'feat: add new invariant\n',
        nonTestFilesChanged: ['src/dashboard.ts'],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -12,3 +12,7 @@ e2e:
   invariants_v2:
     enabled: true
+    structural_invariants:
+      - name: family-member-editable
+        selector: "[data-family-member]"
+        require_affordance: edit
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'v2-08',
        description: 'v2: add window_contracts[] entry → STRENGTHENING (gate passes)',
        filePath: '.autospec/test.yml',
        commitMessages: 'feat: add window contract\nJUSTIFICATION: new API endpoint\n',
        nonTestFilesChanged: ['src/api.ts'],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -12,3 +12,7 @@ e2e:
   invariants_v2:
     enabled: true
+    window_contracts:
+      - name: family-streak-window
+        ui_attribute: data-window-days
+        api_param: from
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'v2-09',
        description: 'v2: widen apply_on_routes → STRENGTHENING (gate passes)',
        filePath: '.autospec/test.yml',
        commitMessages: 'feat: widen route coverage\n',
        nonTestFilesChanged: ['src/archive.ts'],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -5,11 +5,12 @@ e2e:
   invariants_v2:
     enabled: true
     structural_invariants:
       - name: dashboard-done-items-editable
         apply_on_routes:
           - /dashboard
+          - /archive
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'v2-10',
        description: 'v2: raise require_count_at_least → STRENGTHENING (gate passes)',
        filePath: '.autospec/test.yml',
        commitMessages: 'feat: require more items\n',
        nonTestFilesChanged: ['src/dashboard.ts'],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -5,8 +5,8 @@ e2e:
   invariants_v2:
     enabled: true
     structural_invariants:
       - name: dashboard-done-items-editable
-        require_count_at_least: 1
+        require_count_at_least: 5
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'v2-11',
        description: 'v2: raise bfs_max_routes → STRENGTHENING (gate passes)',
        filePath: '.autospec/test.yml',
        commitMessages: 'feat: increase crawl coverage\n',
        nonTestFilesChanged: ['src/routes.ts'],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -12,3 +12,3 @@ e2e:
   invariants_v2:
-    bfs_max_routes: 5
+    bfs_max_routes: 20
`,
        expected: { gate_passed: true, any_bucket: 'STRENGTHENING' },
    },

    {
        id: 'v2-12',
        description: 'v2: change invariant selector (new component) → SHIFTING (justified)',
        filePath: '.autospec/test.yml',
        commitMessages: 'refactor: rename component selector\nJUSTIFICATION: component renamed from task-row to item-row\n',
        nonTestFilesChanged: ['src/components/ItemRow.tsx'],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -5,8 +5,8 @@ e2e:
   invariants_v2:
     enabled: true
     structural_invariants:
       - name: dashboard-done-items-editable
-        selector: "[data-task-row]"
+        selector: "[data-item-row]"
`,
        expected: { gate_passed: true, any_bucket: 'SHIFTING' },
    },

    {
        id: 'v2-13',
        description: 'v2: change path_template in contract_symmetry → SHIFTING (justified)',
        filePath: '.autospec/test.yml',
        commitMessages: 'refactor: update API path\nJUSTIFICATION: API endpoint renamed from /api/timeline to /api/events\n',
        nonTestFilesChanged: ['src/api/events.ts'],
        diff: `diff --git a/.autospec/test.yml b/.autospec/test.yml
index abc..def 100644
--- a/.autospec/test.yml
+++ b/.autospec/test.yml
@@ -5,8 +5,8 @@ e2e:
   invariants_v2:
     enabled: true
     contract_symmetry:
       - name: streak-task-must-be-editable
-        api_endpoint: /api/timeline
+        api_endpoint: /api/events
`,
        expected: { gate_passed: true, any_bucket: 'SHIFTING' },
    },
].map(fixture));

export default FIXTURES;
