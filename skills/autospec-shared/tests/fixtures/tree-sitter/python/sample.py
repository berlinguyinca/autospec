"""sample.py — Python fixture for autospec-docs walker unit tests."""

import os
import json
from pathlib import Path

MAX_RETRIES = 3
DEFAULT_TIMEOUT = 30


def parse_config(file_path: str) -> dict:
    """Parse a JSON config file and return the result."""
    with open(file_path) as f:
        return json.load(f)


def format_message(template: str, **kwargs) -> str:
    """Format a message template with keyword arguments."""
    return template.format(**kwargs)


class ConfigLoader:
    """Loads and caches configuration from disk."""

    def __init__(self, base_dir: str):
        self.base_dir = Path(base_dir)
        self._cache: dict = {}

    def load(self, name: str) -> dict:
        if name not in self._cache:
            self._cache[name] = parse_config(str(self.base_dir / f"{name}.json"))
        return self._cache[name]


if __name__ == "__main__":
    loader = ConfigLoader(os.getcwd())
    print(loader.load("default"))
