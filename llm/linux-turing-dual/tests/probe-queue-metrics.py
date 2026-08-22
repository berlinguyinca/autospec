#!/usr/bin/env python3
"""Diagnostic: does llamacpp:requests_deferred ever rise above zero?

A six-request burst against two slots demonstrably queued -- completions arrived
in pairs -- yet one-second sampling never saw requests_deferred above 0. Either
deferral is shorter-lived than the sample interval, or the metric does not report
what its name suggests.

Sampled at 200 ms with SLOW requests, because a short prompt starts and finishes
between two samples and makes an unusable metric look merely quiet.

    probe-queue-metrics.py [base-url] [model]
"""
import json
import subprocess
import sys
import threading
import time

BASE = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8080"
MODEL = sys.argv[2] if len(sys.argv) > 2 else "qwen3.8-27b"
KEY = open("/tmp/k").read().strip()
N = 6

T0 = time.time()
seen = []
stop = False


def curl(path):
    r = subprocess.run(["curl", "-sf", "-m", "5", "-H", "Authorization: Bearer " + KEY,
                        BASE + path], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


def metric(text, name):
    for line in text.splitlines():
        if line.startswith("llamacpp:" + name):
            return line.split()[-1]
    return None


def sample():
    while not stop:
        m = curl("/metrics?model=" + MODEL)
        s = curl("/slots?model=" + MODEL)
        busy = None
        try:
            busy = sum(1 for x in json.loads(s) if x.get("is_processing"))
        except Exception:
            pass
        tup = (metric(m, "requests_processing"), metric(m, "requests_deferred"), busy)
        if not seen or seen[-1][1] != tup:
            seen.append((round(time.time() - T0, 2), tup))
        time.sleep(0.2)


para = ("Routine log entry. System nominal. Subsystem checks completed without "
        "incident. Telemetry within expected bounds. No operator action required. ")
body = para * int(40000 * 5.7 / len(para))
req = {"model": MODEL,
       "messages": [{"role": "user", "content": body + "\n\nSummarise in one word."}],
       "max_tokens": 32, "temperature": 0,
       "chat_template_kwargs": {"enable_thinking": False}}
with open("/tmp/probe_req.json", "w") as f:
    json.dump(req, f)

done = []


def fire(i):
    t = time.time()
    subprocess.run(["curl", "-s", "-o", "/dev/null", "--max-time", "900",
                    "-H", "Authorization: Bearer " + KEY,
                    "-H", "Content-Type: application/json",
                    BASE + "/v1/chat/completions", "-d", "@/tmp/probe_req.json"])
    done.append((i, round(t - T0, 1), round(time.time() - T0, 1)))


sampler = threading.Thread(target=sample, daemon=True)
sampler.start()
threads = [threading.Thread(target=fire, args=(i,)) for i in range(N)]
for t in threads:
    t.start()
for t in threads:
    t.join()
stop = True
time.sleep(0.5)

print("distinct (processing, deferred, slots_busy) transitions:")
for ts, tup in seen:
    print("  +%7.2fs  processing=%-5s deferred=%-5s slots_busy=%s" % (ts, tup[0], tup[1], tup[2]))
print()
print("request completion order:")
for i, st, en in sorted(done, key=lambda x: x[2]):
    print("  req %d finished +%.1fs" % (i, en))
maxdef = max((int(t[1][1] or 0) for t in seen), default=0)
maxproc = max((int(t[1][0] or 0) for t in seen), default=0)
maxbusy = max((int(t[1][2] or 0) for t in seen), default=0)
print()
print("max requests_processing observed : %d" % maxproc)
print("max requests_deferred observed  : %d" % maxdef)
print("max slots_busy observed         : %d" % maxbusy)
print()
if maxdef > 0:
    print("VERDICT: requests_deferred IS USABLE (peaked at %d)" % maxdef)
elif maxproc > maxbusy:
    print("VERDICT: requests_deferred unusable, but requests_processing (%d) exceeds"
          " slots_busy (%d) -- outstanding can come from requests_processing" % (maxproc, maxbusy))
else:
    print("VERDICT: requests_deferred NOT USABLE and processing never exceeds slots"
          " -- queue depth beyond capacity is NOT observable from llama.cpp")
