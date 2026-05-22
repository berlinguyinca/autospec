"""validator.py — input validator (Python file, second language for multi-lang test)"""

import sys


def validate_name(name: str) -> bool:
    """Check that name is non-empty and contains only alphanumeric characters."""
    return bool(name) and name.isalnum()


def validate_timeout(timeout: int) -> bool:
    """Check that timeout is a positive integer."""
    return isinstance(timeout, int) and timeout > 0


class ConfigValidator:
    """Validates a configuration dictionary."""

    def __init__(self, config: dict):
        self.config = config

    def is_valid(self) -> bool:
        """Return True if all config fields pass validation."""
        return (
            validate_name(self.config.get("name", ""))
            and validate_timeout(self.config.get("timeout", 0))
        )


if __name__ == "__main__":
    cfg = {"name": sys.argv[1] if len(sys.argv) > 1 else "", "timeout": 5000}
    validator = ConfigValidator(cfg)
    print("valid" if validator.is_valid() else "invalid")
    sys.exit(0 if validator.is_valid() else 1)
