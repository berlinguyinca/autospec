#!/usr/bin/env python3
"""Ask two quantisations the same verifiable questions and count disagreements.

    compare-quants.py --a URL,MODEL[,KEY] --b URL,MODEL[,KEY] [--n 40]

"Q8 is better than Q5" has been an assumption in this project since the first
day. It is also the assumption that decides which build to run, so it deserves
a measurement rather than a prior.

What this can and cannot do, stated plainly:

  CAN   detect gross degradation -- a quantisation that has started to break
        instruction-following, arithmetic, or long-context retrieval
  CANNOT resolve a small quality gap. With a few dozen items the confidence
        interval is far wider than the difference between neighbouring
        quantisations, and no amount of careful wording changes that

So it answers "has this build visibly degraded", not "which is better by how
much". Every item is generated here and checked by exact match, so there is no
downloaded dataset to drift and no grader model to introduce its own opinion.
"""
from __future__ import annotations

import argparse
import json
import random
import sys
import urllib.error
import urllib.request

WORDS = ["cobalt", "lantern", "meridian", "quartz", "thistle", "verbena",
         "juniper", "obsidian", "saffron", "wisteria"]


def items(n: int, seed: int = 20260818) -> list[dict]:
    """Deterministic probes with a single checkable answer each."""
    rng = random.Random(seed)
    out = []
    for i in range(n):
        kind = i % 4
        if kind == 0:                                   # arithmetic
            a, b = rng.randint(11, 99), rng.randint(11, 99)
            out.append({"kind": "arith",
                        "q": f"Compute {a} * {b}. Reply with only the number.",
                        "a": str(a * b)})
        elif kind == 1:                                 # instruction following
            w = rng.choice(WORDS)
            out.append({"kind": "format",
                        "q": f"Reply with the word {w!r} reversed, lowercase, "
                             "nothing else.",
                        "a": w[::-1]})
        elif kind == 2:                                 # code reasoning
            xs = [rng.randint(1, 9) for _ in range(5)]
            out.append({"kind": "code",
                        "q": "Given this Python:\n\n"
                             "def f(xs):\n    t = 0\n    for x in xs:\n"
                             "        if x % 2 == 0:\n            t += x\n"
                             "        else:\n            t -= x\n    return t\n\n"
                             f"What is f({xs})? Reply with only the integer.",
                        "a": str(sum(x if x % 2 == 0 else -x for x in xs))})
        else:                                           # retrieval in context
            n_rec = 60
            at = rng.randrange(n_rec)
            code = f"{rng.choice(WORDS).upper()}-{rng.randint(100, 999)}"
            recs = [f"Record {j:03d}: routine entry, nothing of note."
                    for j in range(n_rec)]
            recs[at] = f"Record {at:03d}: authorization code {code} applies."
            out.append({"kind": "recall",
                        "q": "\n".join(recs) +
                             f"\n\nWhat authorization code is in record {at:03d}? "
                             "Reply with only the code.",
                        "a": code})
    return out


def ask(base: str, model: str, key: str, prompt: str,
        thinking: bool = False, max_tokens: int = 24) -> str:
    body = json.dumps({
        "model": model, "temperature": 0, "max_tokens": max_tokens,
        "chat_template_kwargs": {"enable_thinking": thinking},
        "messages": [{"role": "user", "content": prompt}],
    }).encode()
    headers = {"Content-Type": "application/json"}
    if key:
        headers["Authorization"] = f"Bearer {key}"
    req = urllib.request.Request(f"{base}/v1/chat/completions", data=body,
                                 headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=900) as resp:
            d = json.load(resp)
        return d["choices"][0]["message"]["content"].strip()
    except urllib.error.HTTPError as exc:
        # Swallowing the cause turned a slow endpoint into forty identical
        # "<error>" rows that said nothing about what went wrong.
        return f"<http {exc.code}: {exc.read()[:80].decode('utf-8', 'replace')}>"
    except (urllib.error.URLError, OSError, KeyError,
            json.JSONDecodeError) as exc:
        return f"<{type(exc).__name__}: {exc}>"


def norm(s: str) -> str:
    return s.strip().strip(".,'\"` ").lower()


def run(spec: str, probes: list[dict], label: str,
        thinking: bool = False, max_tokens: int = 24) -> dict:
    parts = spec.split(",")
    base, model = parts[0], parts[1]
    key = parts[2] if len(parts) > 2 else ""
    res = {"label": label, "model": model, "by_kind": {}, "answers": []}
    for p in probes:
        got = ask(base, model, key, p["q"], thinking, max_tokens)
        if thinking:
            # With thinking on the answer trails the reasoning; score the last
            # non-empty line so a correct answer is not marked wrong for being
            # preceded by the work.
            tail = [ln for ln in got.splitlines() if ln.strip()]
            got = tail[-1] if tail else got
        ok = norm(got) == norm(p["a"])
        res["answers"].append({"kind": p["kind"], "ok": ok,
                               "want": p["a"], "got": got[:40]})
        k = res["by_kind"].setdefault(p["kind"], [0, 0])
        k[1] += 1
        if ok:
            k[0] += 1
        print(".", end="" if ok else "x", flush=True, file=sys.stderr)
    print("", file=sys.stderr)
    return res


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", required=True, help="URL,MODEL[,KEY]")
    ap.add_argument("--b", required=True, help="URL,MODEL[,KEY]")
    ap.add_argument("--n", type=int, default=40)
    ap.add_argument("--thinking", action="store_true",
                    help="let the model reason before answering. Without this "
                         "a reasoning model fails multi-step arithmetic almost "
                         "always -- and a category both builds fail 90% of has "
                         "no power to tell them apart, which is the entire "
                         "point of running this")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    probes = items(args.n)
    mt = 1024 if args.thinking else 24
    ra = run(args.a, probes, "A", args.thinking, mt)
    rb = run(args.b, probes, "B", args.thinking, mt)

    if args.json:
        print(json.dumps({"a": ra, "b": rb}, indent=2))
        return 0

    def score(r):
        return sum(v[0] for v in r["by_kind"].values())

    print(f"\n{args.n} items, temperature 0, exact match, "
          f"thinking {'on' if args.thinking else 'off'}\n")
    print(f"{'':<10} {ra['model']:<28} {rb['model']:<28}")
    print("-" * 70)
    for kind in ("arith", "format", "code", "recall"):
        a = ra["by_kind"].get(kind, [0, 0])
        b = rb["by_kind"].get(kind, [0, 0])
        print(f"{kind:<10} {f'{a[0]}/{a[1]}':<28} {f'{b[0]}/{b[1]}':<28}")
    sa, sb = score(ra), score(rb)
    print("-" * 70)
    print(f"{'total':<10} {f'{sa}/{args.n}':<28} {f'{sb}/{args.n}':<28}")

    diff = [(p, x, y) for p, x, y in
            zip(probes, ra["answers"], rb["answers"]) if x["ok"] != y["ok"]]
    print(f"\ndisagreements: {len(diff)}")
    for p, x, y in diff[:8]:
        print(f"  [{p['kind']}] want {p['a']!r}")
        print(f"      A {'ok ' if x['ok'] else 'BAD'} {x['got']!r}")
        print(f"      B {'ok ' if y['ok'] else 'BAD'} {y['got']!r}")

    # A few dozen items cannot resolve a small gap; say so rather than let a
    # two-item lead read as a verdict.
    print(f"\nWith n={args.n}, a difference under roughly "
          f"{int(2 * (0.25 * args.n) ** 0.5) + 1} items is not distinguishable "
          "from noise.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
