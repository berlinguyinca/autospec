"""Shared test helpers, under a name no other suite can shadow.

This deliberately is NOT conftest.py. Both node suites are collected in one
pytest run (`pytest llm/linux-turing-dual/tests llm/linux-qwen38/tests`), which
puts BOTH directories on sys.path -- so a test module asking for the bare name
`conftest` got whichever directory came first. It got the other node's, which
has no load_script, and every module here failed to import: 14 collection
errors, the entire suite silently not running in CI while it passed when each
directory was invoked on its own.

A unique module name is the whole fix. There is no conftest.py here any more,
because there is nothing left for one to do.

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

# The scripts import each other by bare name (`import keys as _keys`), which works
# at runtime because the entrypoint puts its own directory on sys.path. Tests must
# reproduce that, or a module's sibling imports would resolve only when some
# earlier load_script call happened to register them -- making collection order
# part of the contract.
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))


def load_script(name: str):
    """Import scripts/<name>.py under the module name <name>."""
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod
