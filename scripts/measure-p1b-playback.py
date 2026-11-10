"""Measure a visible, uninterrupted Windows Release playback session."""

import argparse
import json
import math
import os
from pathlib import Path
import queue
import statistics
import subprocess
import threading
import time


def p95(values):
    return sorted(values)[math.ceil(0.95 * len(values)) - 1] if values else None


def sample_process(process):
    with process.oneshot():
        cpu = process.cpu_times()
        memory = process.memory_info()
        return (time.perf_counter(), cpu.user + cpu.system, memory.private,
                memory.rss, memory.peak_wset, process.io_counters().read_bytes)


def measure(exe, pack, variant, warmup, duration, log, process_factory, page_sizes):
    env = dict(os.environ, DESKTOP_PET_PACK=str(pack.resolve()),
               DESKTOP_PET_FORMAT=variant, DESKTOP_PET_TRACE="1")
    started = time.perf_counter()
    with subprocess.Popen([str(exe.resolve())], env=env, stdout=subprocess.DEVNULL,
                          stderr=subprocess.PIPE, text=True, encoding="utf-8",
                          errors="replace") as child:
        lines = queue.Queue()

        def read():
            for line in child.stderr:
                lines.put((time.perf_counter(), line.rstrip()))
            lines.put((time.perf_counter(), None))

        reader = threading.Thread(target=read)
        reader.start()
        first_asset = first_window = None
        format_verified = False
        frames = []
        resources = []
        try:
            process = process_factory(child.pid) if process_factory else None
            end = started + 30
            while (remaining := end - time.perf_counter()) > 0:
                try:
                    timestamp, line = lines.get(timeout=remaining)
                except queue.Empty:
                    break
                if line is None:
                    raise RuntimeError(f"player exited early; see {log.name}")
                print(line, file=log)
                if line.startswith("First frame presented:"):
                    first_window = timestamp
                if line.startswith("Asset playback:"):
                    format_verified = f"({variant.upper()}," in line
                if line.startswith("First asset frame presented:"):
                    first_asset = timestamp
                    end = first_asset + warmup + duration
                if "using placeholder" in line or "using PNG" in line:
                    raise RuntimeError(f"requested format did not play; see {log.name}")
                if first_asset and timestamp >= first_asset + warmup and line.startswith("P1b frame "):
                    frames.append(tuple(map(int, line.split()[2:])))
                    if process and (not resources or timestamp >= resources[-1][0] + 1):
                        resources.append(sample_process(process))
            if process and resources:
                resources.append(sample_process(process))
        finally:
            child.kill()
            reader.join()
        if first_asset is None or first_window is None or not format_verified:
            raise RuntimeError(f"startup markers or requested format missing; see {log.name}")
        if len(frames) < 2 or frames[-1][2] - frames[0][2] < 0.9 * duration * 1_000_000:
            raise RuntimeError(f"insufficient frame coverage; see {log.name}")
        pairs = list(zip(frames, frames[1:]))
        if any(b[0] <= a[0] for a, b in pairs):
            raise RuntimeError(f"playback sequence reset; see {log.name}")
        expected = frames[-1][0] - frames[0][0] + 1
        result = {
            "window_ms": round((first_window - started) * 1000, 2),
            "asset_ms": round((first_asset - started) * 1000, 2),
            "presented": len(frames),
            "expected": expected,
            "interval_p95_us": p95([b[2] - a[2] for a, b in pairs]),
            "page_switch_prepare_p95_us": p95([b[3] for a, b in pairs if a[1] != b[1]]),
            "page_reuse_prepare_p95_us": p95([
                b[3] for a, b in pairs if a[1] != b[1] and page_sizes[a[1]] == page_sizes[b[1]]
            ]),
            "page_resize_prepare_p95_us": p95([
                b[3] for a, b in pairs if page_sizes[a[1]] != page_sizes[b[1]]
            ]),
        }
        if len(resources) >= 2:
            a, b = resources[0], resources[-1]
            result.update(cpu_percent_one_core=round(100 * (b[1] - a[1]) / (b[0] - a[0]), 2),
                          private_bytes_sampled_max=max(r[2] for r in resources),
                          working_set_bytes_sampled_max=max(r[3] for r in resources),
                          peak_working_set_bytes=b[4], process_read_bytes=b[5] - a[5])
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=Path("target/release/desktop-pet-chatgpt.exe"))
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--formats", nargs="+", choices=("png", "rgba8", "bc7"),
                        default=("png", "rgba8", "bc7"))
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--warmup", type=float, default=120)
    parser.add_argument("--seconds", type=float, default=600)
    parser.add_argument("--process-metrics", action="store_true",
                        help="also collect CPU, memory and process I/O with psutil")
    args = parser.parse_args()
    if args.rounds < 1 or args.warmup < 0 or args.seconds < 1:
        parser.error("require rounds >= 1, warmup >= 0, and seconds >= 1")
    process_factory = None
    if args.process_metrics:
        try:
            from psutil import Process
        except ImportError:
            parser.error("--process-metrics requires psutil")
        process_factory = Process
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = json.loads((args.pack / "manifest.json").read_bytes())
    page_sizes = [page["size"] for page in manifest["pages"]]
    samples = {variant: [] for variant in args.formats}
    for i in range(args.rounds):
        order = list(samples)
        for variant in order[i % len(order):] + order[:i % len(order)]:
            with (args.output / f"{i+1:02d}-{variant}.log").open("x", encoding="utf-8") as log:
                result = measure(args.exe, args.pack, variant, args.warmup, args.seconds,
                                 log, process_factory, page_sizes)
            samples[variant].append(result)
            print(json.dumps(dict(round=i+1, format=variant, **result)), flush=True)
    summary = {}
    for variant, results in samples.items():
        summary[variant] = {
            marker: [min(values), statistics.median(values), max(values)]
            for marker in ("window_ms", "asset_ms")
            for values in [[r[marker] for r in results]]
        }
        summary[variant]["drop_percent"] = (
            100 * sum(r["expected"] - r["presented"] for r in results)
            / sum(r["expected"] for r in results)
        )
    (args.output / "result.json").write_text(
        json.dumps(dict(pack=str(args.pack), warmup=args.warmup, seconds=args.seconds,
                        samples=samples, summary=summary), indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
