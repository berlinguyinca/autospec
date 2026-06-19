#!/usr/bin/env python3
"""
stage_render.py — render QA + contact-sheet stage for autospec-fab.

Uniform stage CLI:
    stage_render.py --in <stl> --out <fragment.json>
                    [--model <metadata.json>] [--render-dir <dir>]

Behaviour (design spec §Release-gate stage 12):
  1. Enumerate the deterministic required view table (render_views.REQUIRED_VIEWS:
     6 axis + 8 corner diagonals + >=3 slicer/print-orientation views, >=16 total).
  2. For each view, call freecad_harness.render() — which subprocesses the
     PATH-resolved freecadcmd (mocked via $TMP/bin in tests) — to emit one
     per-view PNG into the render dir (default .autospec/fab/renders/<model>/).
  3. Assemble ONE deterministic contact sheet WITHOUT any image library: an HTML
     grid (contact-sheet.html) with one <img> per view in table order. This is a
     real artifact the vision stage (#1232) can consume; real PNG compositing
     (PIL) is deferred to the pinned container.
  4. Emit the release-gate fragment {stage,status,detail,findings}:
       - freecadcmd absent  -> status "skip" (no renderer available).
       - all views rendered -> status "pass".
       - any view PNG missing -> status "fail" + finding render_incomplete.

Exit codes: 0 — harness success (verdict lives in fragment "status"); 1 — usage
error. Stdlib only; never imports PIL/trimesh.
"""

from __future__ import annotations

import argparse
import html
import json
import os
import sys

_THIS_DIR = os.path.dirname(os.path.abspath(__file__))
if _THIS_DIR not in sys.path:
    sys.path.insert(0, _THIS_DIR)

import freecad_harness  # noqa: E402
from render_views import REQUIRED_VIEWS  # noqa: E402


def _model_slug(stl_path: str) -> str:
    """Derive a render-subdir slug from the STL filename (sans extension)."""
    base = os.path.basename(stl_path)
    stem = os.path.splitext(base)[0]
    return stem or "model"


def _png_path(render_dir: str, slug: str, view_name: str) -> str:
    """Absolute per-view PNG path: <render_dir>/<slug>/<view_name>.png."""
    return os.path.join(render_dir, slug, view_name + ".png")


def render_all_views(stl_path: str, render_dir: str, slug: str) -> list:
    """
    Render every required view via the harness; return per-view result records.

    Each record: {"name", "kind", "png", "rendered"(bool)}. "rendered" is True
    only when the harness exited 0 AND the PNG file exists on disk afterwards.
    """
    results = []
    for view in REQUIRED_VIEWS:
        png = _png_path(render_dir, slug, view["name"])
        os.makedirs(os.path.dirname(png), exist_ok=True)
        proc = freecad_harness.render(stl_path, view["camera"], png)
        ok = (proc.returncode == 0) and os.path.isfile(png)
        results.append({
            "name": view["name"],
            "kind": view["kind"],
            "png": png,
            "rendered": ok,
        })
    return results


def build_contact_sheet(render_dir: str, slug: str, results: list) -> str:
    """
    Assemble a deterministic HTML contact sheet referencing every view PNG.

    No image library: a fixed CSS grid with one labelled <img> per view, in the
    table order from results. Each <img src> is a relative basename so the sheet
    is portable alongside the PNGs. Returns the absolute sheet path.
    """
    cells = []
    for r in results:
        rel = html.escape(os.path.basename(r["png"]))
        label = html.escape(r["name"])
        marker = "" if r["rendered"] else " (MISSING)"
        cells.append(
            f'<figure><img src="{rel}" alt="{label}" width="320">'
            f"<figcaption>{label}{marker}</figcaption></figure>"
        )
    body = "\n".join(cells)
    sheet_html = (
        "<!doctype html>\n"
        '<html><head><meta charset="utf-8">\n'
        f"<title>contact sheet: {html.escape(slug)}</title>\n"
        "<style>body{margin:0;font-family:sans-serif}"
        ".grid{display:grid;grid-template-columns:repeat(4,1fr);gap:8px;padding:8px}"
        "figure{margin:0}img{width:100%;border:1px solid #ccc}"
        "figcaption{font-size:12px;text-align:center}</style>\n"
        "</head><body>\n"
        f'<h1>{html.escape(slug)} — {len(results)} views</h1>\n'
        f'<div class="grid">\n{body}\n</div>\n'
        "</body></html>\n"
    )
    sheet = os.path.join(render_dir, slug, "contact-sheet.html")
    os.makedirs(os.path.dirname(sheet), exist_ok=True)
    with open(sheet, "w") as f:
        f.write(sheet_html)
    return sheet


def _skip_fragment() -> dict:
    """Fragment for the renderer-absent case (deferred real renders)."""
    return {
        "stage": "render",
        "status": "skip",
        "detail": ("freecadcmd not on PATH; render QA skipped "
                   "(real renders deferred to the pinned container)"),
        "findings": [],
    }


def run_render_stage(stl_path: str, render_dir: str) -> dict:
    """
    Run the render stage and return a release-gate fragment.

    Fragment shape: {stage:"render", status, detail, findings}.
    """
    if freecad_harness.resolve_freecadcmd() is None:
        return _skip_fragment()

    slug = _model_slug(stl_path)
    results = render_all_views(stl_path, render_dir, slug)
    sheet = build_contact_sheet(render_dir, slug, results)

    missing = [r["name"] for r in results if not r["rendered"]]
    detail = (f"{len(results) - len(missing)}/{len(results)} views rendered; "
              f"contact sheet: {sheet}")

    if missing:
        return {
            "stage": "render",
            "status": "fail",
            "detail": detail,
            "findings": [{
                "code": "render_incomplete",
                "message": ("missing render(s) for view(s): "
                            + ", ".join(missing)),
            }],
        }
    return {
        "stage": "render",
        "status": "pass",
        "detail": detail,
        "findings": [],
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="autospec-fab render stage: 16+ view contact sheet"
    )
    parser.add_argument("--in", dest="in_path", required=True,
                        help="Input STL file")
    parser.add_argument("--out", required=True,
                        help="Output release-gate fragment JSON path")
    parser.add_argument("--model", dest="model_path", default=None,
                        help="Per-model metadata sidecar (optional, unused)")
    parser.add_argument("--render-dir", dest="render_dir", default=None,
                        help="Render output dir (default .autospec/fab/renders)")

    args = parser.parse_args(argv)

    render_dir = args.render_dir or os.path.join(
        ".autospec", "fab", "renders")

    fragment = run_render_stage(args.in_path, render_dir)

    out_dir = os.path.dirname(os.path.abspath(args.out))
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, "w") as f:
        json.dump(fragment, f, indent=2)
        f.write("\n")

    return 0


if __name__ == "__main__":
    sys.exit(main())
