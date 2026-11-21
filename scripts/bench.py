#!/usr/bin/env python3
"""End-to-end benchmark: /bin/rm -rf vs an old rmx vs a new rmx.

Usage:
    bench.py <old_rmx> <new_rmx> [runs] [--old-label L] [--root DIR]
             [--json FILE] [--markdown FILE]

Each run regenerates the tree, syncs, then times one deletion (wall clock).
Reports the best of N per tool, in ms, as JSON lines on stdout plus a
markdown table. --json writes the rows as one JSON document, --markdown the
table, --root picks where the trees are built (default: next to this script;
it must be on the file system you want to measure).
"""
import argparse
import json
import os
import platform
import shutil
import subprocess
import time

ap = argparse.ArgumentParser()
ap.add_argument("old")
ap.add_argument("new")
ap.add_argument("runs", nargs="?", type=int, default=5)
ap.add_argument("--old-label", default="rmx 0.7.0")
ap.add_argument("--root", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "bench-tree"))
ap.add_argument("--json")
ap.add_argument("--markdown")
args = ap.parse_args()

OLD_LABEL = args.old_label
ROOT = args.root


def gen_flat(path, n, size):
    os.makedirs(path)
    data = b"x" * size
    for i in range(n):
        with open(os.path.join(path, f"f{i}"), "wb") as f:
            f.write(data)


def gen_node_modules(path):  # 1,000 dirs x 20 files of 100 B
    data = b"x" * 100
    for d in range(1000):
        p = os.path.join(path, f"pkg{d // 50}", f"m{d}")
        os.makedirs(p)
        for i in range(20):
            with open(os.path.join(p, f"f{i}.js"), "wb") as f:
                f.write(data)


def gen_deep(path):  # 200 tops x depth 10, 5 files per level (= 10k files)
    data = b"x" * 100
    for t in range(200):
        p = os.path.join(path, f"t{t}")
        for depth in range(10):
            p = os.path.join(p, f"d{depth}")
            os.makedirs(p)
            for i in range(5):
                with open(os.path.join(p, f"f{i}"), "wb") as f:
                    f.write(data)


WORKLOADS = [
    ("node_modules-like: 1,000 dirs x 20 files", gen_node_modules),
    ("deep: 200 x depth 10, 10k files", gen_deep),
    ("flat: 20,000 files", lambda p: gen_flat(p, 20000, 100)),
    ("flat: 50,000 files", lambda p: gen_flat(p, 50000, 100)),
    ("200 x 5 MB files", lambda p: gen_flat(p, 200, 5 * 1024 * 1024)),
]

TOOLS = [("rm", ["/bin/rm", "-rf"]), (OLD_LABEL, [args.old, "-rfq"]), ("rmx new", [args.new, "-rfq"])]


def main():
    shutil.rmtree(ROOT, ignore_errors=True)
    os.makedirs(ROOT)
    results = []
    for name, gen in WORKLOADS:
        row = {"workload": name}
        for tool, cmd in TOOLS:
            best = None
            for _ in range(args.runs):
                tree = os.path.join(ROOT, "t")
                shutil.rmtree(tree, ignore_errors=True)
                gen(tree)
                os.sync()
                t0 = time.perf_counter()
                subprocess.run(cmd + [tree], check=True)
                ms = (time.perf_counter() - t0) * 1000
                assert not os.path.exists(tree), f"{tool} left {tree}"
                best = ms if best is None else min(best, ms)
            row[tool] = round(best, 1)
        print(json.dumps(row), flush=True)
        results.append(row)

    lines = [
        f"| Workload | rm | {OLD_LABEL} | rmx new | new vs rm | new vs {OLD_LABEL} |",
        "|---|---|---|---|---|---|",
    ]
    for r in results:
        lines.append(
            f"| {r['workload']} | {r['rm']} | {r[OLD_LABEL]} | {r['rmx new']} | "
            f"{r['rm'] / r['rmx new']:.2f}x | {r[OLD_LABEL] / r['rmx new']:.2f}x |"
        )
    table = "\n".join(lines) + "\n"
    print()
    print(table, end="")
    shutil.rmtree(ROOT, ignore_errors=True)

    if args.markdown:
        with open(args.markdown, "w") as f:
            f.write(f"Best of {args.runs} runs, wall clock in ms ({platform.platform()}).\n\n")
            f.write(table)
    if args.json:
        doc = {"runs": args.runs, "platform": platform.platform(), "machine": platform.machine(), "results": results}
        with open(args.json, "w") as f:
            json.dump(doc, f, indent=2)
            f.write("\n")


main()
