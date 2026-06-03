"""Regression tests for ClaudeAdapter.find_transcript project-slug derivation.

Claude Code stores transcripts under ``~/.claude/projects/<slug>`` where the
slug is the absolute cwd with ``/`` and ``.`` replaced by ``-``. Because the
path is absolute, it *starts* with ``/`` and therefore the slug *keeps* a
leading ``-`` (e.g. ``/Users/x/autospec`` -> ``-Users-x-autospec``).

The original implementation did ``cwd.replace("/", "-").lstrip("-")`` which
stripped the leading dash, so every absolute cwd resolved to a directory that
does not exist -> ``TranscriptNotFoundError`` on every PreCompact hook -> the
auto-context-rollover silently no-opped. These tests pin the correct mapping.
"""
from __future__ import annotations

import json

import pytest

from autospec_context_monitor.adapters.base import TranscriptNotFoundError
from autospec_context_monitor.adapters.claude import ClaudeAdapter


def _make_transcript(projects_dir, slug):
    d = projects_dir / slug
    d.mkdir(parents=True)
    t = d / "session.jsonl"
    t.write_text(
        json.dumps(
            {"message": {"model": "claude-sonnet-4-6",
                         "usage": {"input_tokens": 1, "output_tokens": 1}}}
        )
        + "\n",
        encoding="utf-8",
    )
    return t


def test_find_transcript_keeps_leading_dash(tmp_path, monkeypatch):
    """An absolute cwd must resolve to the leading-dash slug Claude actually uses."""
    monkeypatch.setenv("HOME", str(tmp_path))
    projects = tmp_path / ".claude" / "projects"
    # Claude's real directory for /Users/wohlgemuth/IdeaProjects/autospec:
    expected = _make_transcript(projects, "-Users-wohlgemuth-IdeaProjects-autospec")

    found = ClaudeAdapter().find_transcript(
        {"cwd": "/Users/wohlgemuth/IdeaProjects/autospec"}
    )
    assert found == expected


def test_find_transcript_maps_dots_to_dashes(tmp_path, monkeypatch):
    """Dotted path segments (e.g. .claude-worktrees) must map '.' -> '-'."""
    monkeypatch.setenv("HOME", str(tmp_path))
    projects = tmp_path / ".claude" / "projects"
    # /Users/x/repo/.claude-worktrees/wt -> -Users-x-repo--claude-worktrees-wt
    expected = _make_transcript(projects, "-Users-x-repo--claude-worktrees-wt")

    found = ClaudeAdapter().find_transcript(
        {"cwd": "/Users/x/repo/.claude-worktrees/wt"}
    )
    assert found == expected


def test_find_transcript_maps_underscores_and_specials_to_dashes(tmp_path, monkeypatch):
    """Every non-alphanumeric char (underscore, space, …) maps to '-', as Claude does.

    Verified against a live slug: /private/var/folders/m_/hg.../T/x ->
    -private-var-folders-m--hg...-T-x (underscores became dashes).
    """
    monkeypatch.setenv("HOME", str(tmp_path))
    projects = tmp_path / ".claude" / "projects"
    expected = _make_transcript(projects, "-Users-x-my-repo-a-b")

    found = ClaudeAdapter().find_transcript({"cwd": "/Users/x/my_repo/a b"})
    assert found == expected


def test_find_transcript_missing_still_raises(tmp_path, monkeypatch):
    """When no transcript exists under the correct slug, raise (not silently pass)."""
    monkeypatch.setenv("HOME", str(tmp_path))
    (tmp_path / ".claude" / "projects").mkdir(parents=True)
    with pytest.raises(TranscriptNotFoundError):
        ClaudeAdapter().find_transcript({"cwd": "/no/such/repo"})
