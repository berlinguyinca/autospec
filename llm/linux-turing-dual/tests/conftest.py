"""Shared test helpers.

`load_script` exists because these scripts are executables with hyphens and no
package, so they cannot be imported normally. It registers the module in
sys.modules BEFORE executing it, exactly as a real import does -- `from
__future__ import annotations` makes @dataclass resolve field types through
sys.modules[cls.__module__], and a module loaded by path alone dies at class
creation with "NoneType has no attribute __dict__".

Deliberately no collect_ignore here: every test file in this directory is a
pytest file and should be collected.
"""
import importlib.util
import pathlib
import sys

SCRIPTS = pathlib.Path(__file__).resolve().parent.parent / "scripts"


def load_script(name: str):
    """Import scripts/<name>.py under the module name <name>."""
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod
