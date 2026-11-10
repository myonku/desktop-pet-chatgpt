"""Measure a visible, uninterrupted Windows Release playback session."""

import argparse
import hashlib
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
                memory.rss, memory.peak_wset, process.io_counters().read_bytes,
                process.num_ctx_switches().voluntary)


class GpuCounters:
    """Read per-PID WDDM engine and memory counters through Windows PDH."""

    def __init__(self):
        import ctypes
        from ctypes import wintypes

        class Item(ctypes.Structure):
            _fields_ = [("name", wintypes.LPWSTR), ("status", wintypes.DWORD),
                        ("value", ctypes.c_double)]

        self.ctypes = ctypes
        self.item = Item
        self.pdh = ctypes.WinDLL("pdh")
        self.query = wintypes.HANDLE()
        self.counters = {}
        self.check(self.pdh.PdhOpenQueryW(None, 0, ctypes.byref(self.query)))
        try:
            for key, path in {
                "engine_percent": r"\GPU Engine(*)\Utilization Percentage",
                "shared_bytes": r"\GPU Process Memory(*)\Shared Usage",
                "dedicated_bytes": r"\GPU Process Memory(*)\Dedicated Usage",
            }.items():
                counter = wintypes.HANDLE()
                self.check(self.pdh.PdhAddCounterW(self.query, ctypes.c_wchar_p(path), 0,
                                                   ctypes.byref(counter)))
                self.counters[key] = counter
            self.collect()
        except BaseException:
            self.close()
            raise

    @staticmethod
    def check(status):
        if status:
            raise OSError(f"PDH status 0x{status & 0xffffffff:08x}")

    def collect(self):
        self.check(self.pdh.PdhCollectQueryData(self.query))

    def sample(self, pid):
        ctypes = self.ctypes
        self.collect()
        metrics = {}
        for key, counter in self.counters.items():
            get_array = self.pdh.PdhGetFormattedCounterArrayW
            more_data = 0x800007d2  # PDH_MORE_DATA
            metrics[key] = {}
            for _ in range(3):
                size, count = ctypes.c_uint32(), ctypes.c_uint32()
                args = (counter, 0x200, ctypes.byref(size), ctypes.byref(count))  # PDH_FMT_DOUBLE
                status = get_array(*args, None)
                if status & 0xffffffff != more_data:
                    self.check(status)
                buf = ctypes.create_string_buffer(size.value)
                status = get_array(*args, buf)
                if status & 0xffffffff == more_data:
                    continue  # Instances can change between the two calls.
                self.check(status)
                items = (self.item * count.value).from_buffer(buf)
                metrics[key] = {item.name: item.value for item in items
                                if item.status in (0, 1) and item.name.startswith(f"pid_{pid}_")}
                break
        return metrics

    def close(self):
        self.pdh.PdhCloseQuery(self.query)


def measure(exe, pack, variant, warmup, duration, log, process_factory, page_sizes, reference,
            gpu_factory):
    env = dict(os.environ, DESKTOP_PET_PACK=str(pack.resolve()),
               DESKTOP_PET_FORMAT=variant, DESKTOP_PET_TRACE="1")
    if variant == "ffmpeg":
        env.update(DESKTOP_PET_REFERENCE_FFMPEG=str(reference[0].resolve()),
                   DESKTOP_PET_REFERENCE_VIDEO=str(reference[1].resolve()))
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
        decoder_resources = []
        decoder = None
        gpu = None
        gpu_samples = []
        try:
            process = process_factory(child.pid) if process_factory else None

            def sample():
                nonlocal decoder
                resources.append(sample_process(process))
                if variant == "ffmpeg":
                    if decoder is None:
                        children = process.children()
                        if len(children) != 1:
                            raise RuntimeError(f"expected one FFmpeg child; see {log.name}")
                        decoder = children[0]
                    decoder_resources.append(sample_process(decoder))
                if gpu:
                    gpu_samples.append(gpu.sample(child.pid))

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
                    log.flush()
                    if gpu_factory:
                        gpu = gpu_factory()
                if ("using placeholder" in line or "using PNG" in line
                        or line.startswith("Asset playback format fallback:")):
                    raise RuntimeError(f"requested format did not play; see {log.name}")
                if first_asset and timestamp >= first_asset + warmup and line.startswith("P1b frame "):
                    frames.append(tuple(map(int, line.split()[2:])))
                    if process and (not resources or timestamp >= resources[-1][0] + 1):
                        sample()
            if process and resources:
                sample()
        finally:
            if gpu:
                gpu.close()
            if variant == "ffmpeg":
                # A forced parent exit cannot run the Rust decoder's Drop handler.
                import psutil
                try:
                    descendants = psutil.Process(child.pid).children(recursive=True)
                except psutil.NoSuchProcess:
                    descendants = []
                for descendant in descendants:
                    try:
                        descendant.kill()
                    except psutil.NoSuchProcess:
                        pass
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
        def summarize(samples):
            a, b = samples[0], samples[-1]
            return dict(cpu_percent_one_core=round(100 * (b[1] - a[1]) / (b[0] - a[0]), 2),
                        private_bytes_start=a[2], private_bytes_end=b[2],
                        private_bytes_sampled_max=max(r[2] for r in samples),
                        working_set_bytes_sampled_max=max(r[3] for r in samples),
                        peak_working_set_bytes=b[4], process_read_bytes=b[5] - a[5],
                        context_switches_per_s=round((b[6] - a[6]) / (b[0] - a[0]), 2))

        if len(resources) >= 2:
            result.update(summarize(resources))
        if len(decoder_resources) >= 2:
            result["decoder"] = summarize(decoder_resources)
        if gpu_samples:
            active = [row["engine_percent"] for row in gpu_samples if row["engine_percent"]]
            engines = sorted({name for row in active for name in row if not name.endswith("_engtype_")})
            memory = {key: [sum(row[key].values()) for row in gpu_samples if row[key]]
                      for key in ("shared_bytes", "dedicated_bytes")}
            result["gpu"] = {
                "engine_mean_percent": {name.split("_", 2)[2]: round(statistics.mean(
                    row.get(name, 0) for row in active), 4) for name in engines},
                "engine_sample_count": len(active),
                **{key: round(max(values)) if values else None for key, values in memory.items()},
            }
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--pack", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--formats", nargs="+", choices=("png", "rgba8", "bc7", "ffmpeg"),
                        default=("png", "rgba8", "bc7"))
    parser.add_argument("--ffmpeg", type=Path)
    parser.add_argument("--video", type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--warmup", type=float, default=120)
    parser.add_argument("--seconds", type=float, default=600)
    parser.add_argument("--process-metrics", action="store_true",
                        help="also collect CPU, memory and process I/O with psutil")
    parser.add_argument("--gpu-metrics", action="store_true",
                        help="also collect WDDM GPU engine and memory counters")
    args = parser.parse_args()
    if args.rounds < 1 or args.warmup < 0 or args.seconds < 1:
        parser.error("require rounds >= 1, warmup >= 0, and seconds >= 1")
    process_factory = None
    if "ffmpeg" in args.formats and (args.ffmpeg is None or args.video is None):
        parser.error("ffmpeg format requires --ffmpeg and --video")
    if args.process_metrics or args.gpu_metrics or "ffmpeg" in args.formats:
        try:
            from psutil import Process
        except ImportError:
            parser.error("--process-metrics and ffmpeg format require psutil")
        process_factory = Process
    args.output.mkdir(parents=True, exist_ok=False)
    manifest_bytes = (args.pack / "manifest.json").read_bytes()
    manifest = json.loads(manifest_bytes)
    if "ffmpeg" in args.formats:
        with args.video.open("rb") as video:
            video_sha256 = hashlib.file_digest(video, "sha256").hexdigest()
        if video_sha256 not in manifest["source_sha256"]:
            parser.error("--video does not match a source hash in the pack")
    page_sizes = [page["size"] for page in manifest["pages"]]
    samples = {variant: [] for variant in args.formats}
    metadata = dict(pack=str(args.pack), manifest_sha256=hashlib.sha256(manifest_bytes).hexdigest(),
                    exe_sha256=hashlib.sha256(args.exe.read_bytes()).hexdigest(),
                    warmup=args.warmup, seconds=args.seconds, rounds=args.rounds, formats=args.formats)
    if "ffmpeg" in args.formats:
        metadata["video_sha256"] = video_sha256
    (args.output / "run.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    for i in range(args.rounds):
        order = list(samples)
        for variant in order[i % len(order):] + order[:i % len(order)]:
            with (args.output / f"{i+1:02d}-{variant}.log").open("x", encoding="utf-8") as log:
                result = measure(args.exe, args.pack, variant, args.warmup, args.seconds,
                                 log, process_factory, page_sizes, (args.ffmpeg, args.video),
                                 GpuCounters if args.gpu_metrics else None)
            samples[variant].append(result)
            row = json.dumps(dict(round=i+1, format=variant, **result))
            with (args.output / "rounds.jsonl").open("a", encoding="utf-8") as results:
                print(row, file=results)
            print(row, flush=True)
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
        json.dumps(dict(**metadata, samples=samples, summary=summary), indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
