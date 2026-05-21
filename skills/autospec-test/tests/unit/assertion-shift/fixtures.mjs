// fixtures.mjs — fixture corpus for assertion-shift classifier tests.
// Each fixture: { id, description, diff, filePath, commitMessages, nonTestFilesChanged, expected }
// expected: { gate_passed, verdicts: [{bucket}] } (verdicts may be partial)

export const FIXTURES = [

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
        description: 'jest: test.skip added → LOOSENING',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'fix: skip flaky\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -1,7 +1,7 @@
-test('add', () => {
+test.skip('add', () => {
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
        description: 'pytest: pytest.mark.skip added → LOOSENING',
        filePath: 'tests/test_api.py',
        commitMessages: 'fix: skip\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_api.py b/tests/test_api.py
@@ -1,4 +1,5 @@
+@pytest.mark.skip
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
        description: 'jest: test.skip removed → STRENGTHENING',
        filePath: 'src/__tests__/calc.test.js',
        commitMessages: 'test: unskip\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/src/__tests__/calc.test.js b/src/__tests__/calc.test.js
@@ -1,7 +1,7 @@
-test.skip('add', () => {
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
        description: 'pytest: pytest.mark.skip removed → STRENGTHENING',
        filePath: 'tests/test_api.py',
        commitMessages: 'test: unskip\n',
        nonTestFilesChanged: [],
        diff: `diff --git a/tests/test_api.py b/tests/test_api.py
@@ -1,4 +1,3 @@
-@pytest.mark.skip
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
];

export default FIXTURES;
