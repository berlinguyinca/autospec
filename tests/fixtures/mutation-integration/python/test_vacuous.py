# Deliberate zero-assertion test fixture for Python.
# The test below has no assert/expect call — density floor should flag it.
# Used by integration test 8 in tests/mutation-integration.bats.

def test_always_passes():
    """This test has no assertions — it always passes regardless of behavior."""
    x = 1 + 1
    # Missing: assert x == 2
