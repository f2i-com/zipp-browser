"""Run Web Platform Tests CSS reftests against zipp-browser.

A reftest is a page and a reference that must render the same (or, for a
`mismatch`, differently). zipp-browser renders both (`--reftest`) and compares
the pixels, allowing what the test's `fuzzy` annotation allows. This serves a
WPT checkout over HTTP (tests load `/fonts/Ahem.ttf` and the like by absolute
path), finds the reftests in the directories given, runs them in parallel
shards -- restarting a shard where it stopped when a test crashes the browser
or hangs -- and writes a summary.

    python tools/wpt_reftest.py --wpt ../wpt --exe target/release/zipp-browser.exe \\
        --out wpt-results css/css-flexbox css/CSS2/floats

A sparse WPT checkout is enough:

    git clone --depth 1 --filter=blob:none --sparse https://github.com/web-platform-tests/wpt
    cd wpt && git sparse-checkout set css/css-flexbox css/reference css/support fonts resources

Results: `<out>/results.tsv` (test, status, differing pixels, largest
difference), `<out>/summary.json` and `<out>/summary.md`. A rerun with the same
`--out` skips the tests already done; delete the folder to start over.
"""

import argparse
import functools
import html.parser
import http.server
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.parse

TEST_EXTENSIONS = (".html", ".htm", ".xht", ".xhtml")


class Links(html.parser.HTMLParser):
    """The reference links and fuzzy annotations of one test."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.refs = []  # (href, "match" | "mismatch")
        self.fuzzy = []  # content strings

    def handle_starttag(self, tag, attrs):
        a = {k.lower(): (v or "") for k, v in attrs}
        if tag == "link" and a.get("rel", "").lower() in ("match", "mismatch") and a.get("href"):
            self.refs.append((a["href"], a["rel"].lower()))
        elif tag == "meta" and a.get("name", "").lower() == "fuzzy":
            self.fuzzy.append(a.get("content", ""))


def upper(value):
    """The upper bound of a fuzzy range: `0-2` -> 2, `5` -> 5."""
    value = value.strip()
    return int(value.split("-")[-1]) if value else 0


def fuzzy_allowance(contents, ref_name):
    """WPT's `fuzzy` annotation, for this reference: (largest difference, pixels)."""
    best = (0, 0)
    for content in contents:
        # `ref.html:maxDifference=1;totalPixels=10` applies to that reference only.
        m = re.match(r"^\s*([^=;:]+):(.*)$", content)
        scope, rule = (m.group(1).strip(), m.group(2)) if m else ("", content)
        if scope and os.path.basename(scope) != ref_name:
            continue
        parts = [p.strip() for p in rule.split(";") if p.strip()]
        diff = pixels = 0
        for i, part in enumerate(parts):
            key, _, value = part.partition("=")
            if not value:
                # Positional form: `0-2;0-300`.
                value, key = key, ("maxDifference" if i == 0 else "totalPixels")
            if key.strip() == "maxDifference":
                diff = upper(value)
            elif key.strip() == "totalPixels":
                pixels = upper(value)
        best = (max(best[0], diff), max(best[1], pixels))
    return best


def find_tests(wpt, dirs):
    """Every reftest under `dirs`, as (test path, reference path, diff, pixels, kind)."""
    tests = []
    for d in dirs:
        for root, subdirs, files in os.walk(os.path.join(wpt, d)):
            subdirs[:] = [s for s in subdirs if s not in ("support", "reference", "resources", "tools")]
            for name in sorted(files):
                if not name.endswith(TEST_EXTENSIONS) or "-ref" in name or name.startswith("ref"):
                    continue
                path = os.path.join(root, name)
                try:
                    with open(path, encoding="utf-8", errors="replace") as f:
                        text = f.read()
                except OSError:
                    continue
                if "match" not in text:
                    continue
                parser = Links()
                try:
                    parser.feed(text)
                except Exception:
                    continue
                if not parser.refs:
                    continue
                href, kind = parser.refs[0]
                href = href.split("#")[0].split("?")[0]
                ref = os.path.normpath(os.path.join(wpt, href.lstrip("/")) if href.startswith("/") else os.path.join(root, href))
                if not os.path.exists(ref):
                    continue
                diff, pixels = fuzzy_allowance(parser.fuzzy, os.path.basename(ref))
                tests.append((path, ref, diff, pixels, kind))
    return tests


def serve(wpt):
    """Serve the checkout on a free port, quietly; returns the base URL."""

    class Quiet(http.server.SimpleHTTPRequestHandler):
        # Connections kept open and reused: one per request, thousands a
        # minute, left closed connections holding ports, and new ones failed.
        protocol_version = "HTTP/1.1"

        def log_message(self, *args):
            pass

    class Server(http.server.ThreadingHTTPServer):
        # The default backlog of 5 refused connections under several shards'
        # requests at once, and a page that failed to load failed its test.
        request_queue_size = 512
        daemon_threads = True

    Quiet.extensions_map.update({".xht": "application/xhtml+xml", ".xhtml": "application/xhtml+xml"})
    server = Server(("127.0.0.1", 0), functools.partial(Quiet, directory=wpt))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/"


def url_of(base, wpt, path):
    rel = os.path.relpath(path, wpt).replace(os.sep, "/")
    return base + urllib.parse.quote(rel)


def run_shard(exe, pairs_path, results_path, total, hang_seconds, log):
    """Run one shard to the end: restart where it stopped after a crash, and
    record the test it was on as `crash`, or as `timeout` if it hung."""
    with open(pairs_path, encoding="utf-8") as f:
        order = [line.split("\t")[0] for line in f.read().splitlines() if line]
    while True:
        done = set()
        if os.path.exists(results_path):
            with open(results_path, encoding="utf-8") as f:
                done = {line.split("\t")[0] for line in f.read().splitlines() if line}
        pending = [t for t in order if t not in done]
        if not pending:
            return
        proc = subprocess.Popen([exe, "--reftest", pairs_path, results_path],
                                stdout=subprocess.DEVNULL, stderr=log)
        size = os.path.getsize(results_path) if os.path.exists(results_path) else 0
        last_change = time.time()
        status = None
        while proc.poll() is None:
            time.sleep(0.5)
            now_size = os.path.getsize(results_path) if os.path.exists(results_path) else 0
            if now_size != size:
                size, last_change = now_size, time.time()
            elif time.time() - last_change > hang_seconds:
                proc.kill()
                status = "timeout"
                break
        proc.wait()
        # Whatever it was on when it stopped: crashed, or hung.
        done_now = set()
        if os.path.exists(results_path):
            with open(results_path, encoding="utf-8") as f:
                done_now = {line.split("\t")[0] for line in f.read().splitlines() if line}
        still = [t for t in order if t not in done_now]
        if still and (status or proc.returncode != 0):
            with open(results_path, "a", encoding="utf-8") as f:
                f.write(f"{still[0]}\t{status or 'crash'}\t0\t0\n")
        elif still and proc.returncode == 0:
            return  # It ended normally without them: unreadable pairs.


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--wpt", required=True, help="the WPT checkout")
    ap.add_argument("--exe", required=True, help="zipp-browser")
    ap.add_argument("--out", required=True, help="a folder for the results")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) // 3))
    ap.add_argument("--hang", type=float, default=60, help="seconds without progress before a shard is restarted")
    ap.add_argument("--limit", type=int, default=0, help="run only the first N tests (0: all)")
    ap.add_argument("dirs", nargs="+", help="directories in the checkout, such as css/css-flexbox")
    args = ap.parse_args()

    wpt = os.path.abspath(args.wpt)
    os.makedirs(args.out, exist_ok=True)
    tests = find_tests(wpt, args.dirs)
    if args.limit:
        tests = tests[: args.limit]
    base = serve(wpt)
    print(f"{len(tests)} reftests; serving {wpt} at {base}; {args.jobs} jobs", flush=True)

    # Shards by turns, so each gets a share of every directory.
    shards = [tests[i :: args.jobs] for i in range(args.jobs)]
    threads = []
    t0 = time.time()
    log = open(os.path.join(args.out, "browser.log"), "ab")
    for i, shard in enumerate(shards):
        pairs = os.path.join(args.out, f"pairs-{i}.tsv")
        with open(pairs, "w", encoding="utf-8") as f:
            for test, ref, diff, pixels, kind in shard:
                f.write(f"{url_of(base, wpt, test)}\t{url_of(base, wpt, ref)}\t{diff}\t{pixels}\t{kind}\n")
        results = os.path.join(args.out, f"results-{i}.tsv")
        th = threading.Thread(target=run_shard, args=(os.path.abspath(args.exe), pairs, results, len(shard), args.hang, log))
        th.start()
        threads.append(th)
    while any(t.is_alive() for t in threads):
        time.sleep(5)
        done = 0
        for i in range(args.jobs):
            p = os.path.join(args.out, f"results-{i}.tsv")
            if os.path.exists(p):
                with open(p, encoding="utf-8") as f:
                    done += sum(1 for _ in f)
        print(f"  {done}/{len(tests)} after {time.time() - t0:.0f}s", flush=True)

    # Gather, by the base URL's path.
    results = {}
    for i in range(args.jobs):
        p = os.path.join(args.out, f"results-{i}.tsv")
        if os.path.exists(p):
            with open(p, encoding="utf-8") as f:
                for line in f.read().splitlines():
                    parts = line.split("\t")
                    if len(parts) >= 2:
                        rel = urllib.parse.unquote(parts[0].split("/", 3)[-1])
                        results[rel] = parts[1:]
    with open(os.path.join(args.out, "results.tsv"), "w", encoding="utf-8") as f:
        for rel in sorted(results):
            f.write(rel + "\t" + "\t".join(results[rel]) + "\n")

    def area(rel):
        parts = rel.split("/")
        return "/".join(parts[:3]) if len(parts) > 3 and parts[1] == "CSS2" else "/".join(parts[:2])

    summary = {}
    for rel, r in results.items():
        s = summary.setdefault(area(rel), {"pass": 0, "fail": 0, "error": 0, "crash": 0, "timeout": 0})
        s[r[0]] = s.get(r[0], 0) + 1
    total_pass = sum(s["pass"] for s in summary.values())
    with open(os.path.join(args.out, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=1, sort_keys=True)
    lines = ["| Area | Pass | Total | Rate |", "|---|---|---|---|"]
    for name in sorted(summary):
        s = summary[name]
        n = sum(s.values())
        lines.append(f"| {name} | {s['pass']} | {n} | {100 * s['pass'] / max(n, 1):.1f}% |")
    lines.append(f"| **all** | **{total_pass}** | **{len(results)}** | **{100 * total_pass / max(len(results), 1):.1f}%** |")
    with open(os.path.join(args.out, "summary.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    print("\n".join(lines))
    print(f"done in {time.time() - t0:.0f}s")


if __name__ == "__main__":
    sys.exit(main())
