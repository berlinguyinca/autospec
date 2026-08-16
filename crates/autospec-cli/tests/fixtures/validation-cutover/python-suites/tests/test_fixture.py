import os


def test_fixture_passes():
    assert os.environ["PYTHONDONTWRITEBYTECODE"] == "1"
    assert os.environ["PYTEST_DISABLE_PLUGIN_AUTOLOAD"] == "1"
