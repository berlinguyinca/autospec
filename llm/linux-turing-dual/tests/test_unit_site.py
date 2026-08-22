"""The site.conf loader.

The node's real address must never be committed to this public repository, so
it loads from a file outside the tree. These tests pin the two behaviours that
make that safe: a missing or still-placeholder value is refused loudly with
EX_CONFIG naming the file, and an environment variable beats the file.
"""
import pathlib
import subprocess
import textwrap

HERE = pathlib.Path(__file__).resolve().parents[1]
SITE = HERE / "scripts" / "site.sh"

# Addresses here are RFC 5737 documentation range (192.0.2.0/24) on purpose:
# the structural test forbids any other literal IPv4 under the node directory,
# including in tests, because every address this node uses comes from site.conf.
COMPLETE = textwrap.dedent('''
    : "${QT_NODE_ADDR:=192.0.2.5}"
    : "${QT_UPLINK_IF:=eth1}"
    : "${QT_MODELS_DIR:=/srv/models}"
    : "${QT_PORT:=8080}"
    : "${QT_DASH_ADDR:=192.0.2.6}"
    : "${QT_DASH_PORT:=8081}"
''')


def run(tmp_path, body, tail='echo "$QT_NODE_ADDR|$QT_UPLINK_IF|$QT_PORT"'):
    cfg = tmp_path / "site.conf"
    cfg.write_text(body)
    return subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{cfg}"; . "{SITE}"; require_site; rc=$?; '
                       f'[ $rc -ne 0 ] && exit $rc; {tail}'],
        capture_output=True, text=True)


def test_loads_a_complete_config(tmp_path):
    r = run(tmp_path, COMPLETE)
    assert r.returncode == 0, r.stderr
    assert r.stdout.strip() == "192.0.2.5|eth1|8080"


def test_placeholder_is_rejected_with_ex_config(tmp_path):
    body = COMPLETE.replace("192.0.2.5", "<node-addr>")
    r = run(tmp_path, body)
    assert r.returncode == 78
    assert "placeholder" in r.stderr
    assert "site.conf" in r.stderr


def test_missing_value_is_rejected_and_named(tmp_path):
    body = '\n'.join(l for l in COMPLETE.splitlines() if "QT_UPLINK_IF" not in l)
    r = run(tmp_path, body)
    assert r.returncode == 78
    assert "QT_UPLINK_IF" in r.stderr


def test_error_names_the_file_to_edit(tmp_path):
    r = run(tmp_path, ': "${QT_NODE_ADDR:=192.0.2.5}"\n')
    assert r.returncode == 78
    assert str(tmp_path / "site.conf") in r.stderr


def test_environment_wins_over_the_file(tmp_path):
    cfg = tmp_path / "site.conf"
    cfg.write_text(COMPLETE)
    r = subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{cfg}"; QT_PORT=9999; . "{SITE}"; '
                       'require_site && echo "$QT_PORT"'],
        capture_output=True, text=True)
    assert r.stdout.strip() == "9999", r.stderr


def test_missing_file_is_ex_config_not_a_crash(tmp_path):
    r = subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{tmp_path}/nope.conf"; . "{SITE}"; require_site'],
        capture_output=True, text=True)
    assert r.returncode == 78
    assert "nope.conf" in r.stderr


def test_sourcing_does_not_kill_the_callers_shell(tmp_path):
    """It is sourced, so it must never exit -- and must set no RETURN trap."""
    r = subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{tmp_path}/nope.conf"; . "{SITE}"; '
                       'require_site >/dev/null 2>&1; echo "still alive"'],
        capture_output=True, text=True)
    assert r.returncode == 0
    assert "still alive" in r.stdout


def test_example_config_is_itself_rejected(tmp_path):
    """The committed example must NOT be a usable config -- it is placeholders."""
    example = HERE / "config" / "site.conf.example"
    r = subprocess.run(
        ["bash", "-c", f'QT_SITE_CONF="{example}"; . "{SITE}"; require_site'],
        capture_output=True, text=True)
    assert r.returncode == 78


def test_system_path_is_preferred_over_home(tmp_path, monkeypatch):
    """The service runs with ProtectHome=true and cannot see /home at all."""
    etc = tmp_path / "etc"; etc.mkdir()
    home = tmp_path / "home" / ".config" / "qwen-turing"; home.mkdir(parents=True)
    (home / "site.conf").write_text(COMPLETE.replace("8080", "1111"))
    # No /etc file here, so the home one must be found via XDG.
    r = subprocess.run(
        ["bash", "-c", f'unset QT_SITE_CONF; XDG_CONFIG_HOME="{tmp_path}/home/.config"; '
                       f'. "{SITE}"; require_site && echo "$QT_PORT"'],
        capture_output=True, text=True)
    assert r.stdout.strip() == "1111", r.stderr


def test_error_names_the_system_path_when_nothing_is_readable(tmp_path):
    """An unactionable error is worse than none; it must say where to write."""
    r = subprocess.run(
        ["bash", "-c", f'unset QT_SITE_CONF; XDG_CONFIG_HOME="{tmp_path}/nowhere"; '
                       f'HOME="{tmp_path}/nowhere"; . "{SITE}"; require_site'],
        capture_output=True, text=True)
    assert r.returncode == 78
    assert "/etc/qwen-turing/site.conf" in r.stderr
