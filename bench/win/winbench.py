#!/usr/bin/env python3
"""Repeatable local benchmark of CutOut's background removal on Windows.

    python bench/win/winbench.py --cli <cutout-cli.exe> --models <dir> --images <dir> --out <dir> [--runs 3]
                                 [--config gpu cpu legacy-gpu legacy-cpu]

For every configuration the benchmark CLI (`cutout-cli bench`, built with `--features directml`) is
run `--runs` times over all images. Reported per configuration:

* model load time (median of the runs),
* inference, pre- and post-processing time per image: median, 90th percentile and maximum over
  every image of every run (the first, discarded warm-up prediction of each run is not included),
* peak working set and peak private bytes of the process (50 ms samples),
* peak video memory of the NVIDIA card (`nvidia-smi`, whole card minus its use before the run;
  empty on PCs without one).

Results go to `<out>/winbench.json`; the cutouts stay in `<out>/<config>/` for scoring with
`cutout-cli score` / `parity`. Needs `pip install psutil`. Run it on an idle machine: other work
changes the CPU numbers by 20 % and more.

Configurations: gpu / cpu use the v2 package (birefnet-general-v2-{gpu,cpu}.onnx + .weights);
legacy-gpu / legacy-cpu use the onnx-community exports (birefnet-general-fp16.onnx / -fp32.onnx),
which is the comparison baseline. Missing model files are skipped.
"""
import argparse
import json
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

import psutil

CONFIGS = {
    "gpu": ("birefnet-general-v2-gpu.onnx", "directml"),
    "cpu": ("birefnet-general-v2-cpu.onnx", "cpu"),
    "legacy-gpu": ("birefnet-general-fp16.onnx", "directml"),
    "legacy-cpu": ("birefnet-general-fp32.onnx", "cpu"),
}


def gpu_used_mib():
    try:
        out = subprocess.run(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"], capture_output=True, text=True, timeout=5).stdout
        return int(out.strip().splitlines()[0])
    except Exception:
        return None


def measured_run(cmd):
    """Run `cmd`; return (exit code, wall seconds, peak working set, peak private bytes, peak GPU MiB)."""
    base = gpu_used_mib()
    peak = {"ws": 0, "private": 0, "gpu": base or 0}
    poller = None
    if base is not None:
        poller = subprocess.Popen(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits", "-lms", "100"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, text=True)

        def read():
            for line in poller.stdout:
                try:
                    peak["gpu"] = max(peak["gpu"], int(line.strip()))
                except ValueError:
                    pass

        threading.Thread(target=read, daemon=True).start()
        time.sleep(0.3)
    t0 = time.perf_counter()
    proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ps = psutil.Process(proc.pid)
    while proc.poll() is None:
        try:
            m = ps.memory_info()
            peak["ws"] = max(peak["ws"], m.rss)
            peak["private"] = max(peak["private"], getattr(m, "private", 0))
        except psutil.Error:
            pass
        time.sleep(0.05)
    wall = time.perf_counter() - t0
    if poller:
        time.sleep(0.25)
        poller.kill()
    return proc.returncode, wall, peak["ws"], peak["private"], None if base is None else peak["gpu"] - base


def pct(values, q):
    s = sorted(values)
    return s[min(len(s) - 1, int(round(q * (len(s) - 1))))]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cli", required=True, type=Path)
    ap.add_argument("--models", required=True, type=Path)
    ap.add_argument("--images", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--config", nargs="+", default=list(CONFIGS), choices=list(CONFIGS))
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    report = {}
    for name in args.config:
        model, accel = CONFIGS[name]
        path = args.models / model
        if not path.is_file():
            print(f"{name}: {path} not found, skipped")
            continue
        out = args.out / name
        loads, inf, pre, post, total, ws, private, gpu = [], [], [], [], [], [], [], []
        failed = None
        for _ in range(args.runs):
            code, wall, w, p, g = measured_run([str(args.cli), "bench", str(path), accel, str(args.images), str(out)])
            if code != 0:
                failed = f"exit code {code}"
                break
            rep = json.loads((out / f"report_{path.stem}_{accel}.json").read_text())
            loads.append(rep["load_ms"])
            for r in rep["results"]:
                inf.append(r["inference_ms"])
                pre.append(r.get("pre_ms", 0))
                post.append(r["post_ms"])
                total.append(r["total_ms"])
            ws.append(w)
            private.append(p)
            gpu.append(g)
        if failed:
            print(f"{name}: FAILED ({failed})")
            report[name] = {"failed": failed}
            continue
        row = {
            "model": model,
            "accel": accel,
            "runs": args.runs,
            "samples": len(inf),
            "load_ms": statistics.median(loads),
            "inference_ms": {"median": statistics.median(inf), "p90": pct(inf, 0.9), "max": max(inf)},
            "pre_ms": {"median": statistics.median(pre), "max": max(pre)},
            "post_ms": {"median": statistics.median(post), "max": max(post)},
            "total_ms": {"median": statistics.median(total), "p90": pct(total, 0.9), "max": max(total)},
            "peak_working_set_mb": round(max(ws) / 1e6),
            "peak_private_mb": round(max(private) / 1e6),
            "peak_gpu_mib": None if gpu[0] is None else max(gpu),
        }
        report[name] = row
        print(f"{name:11s} load {row['load_ms']:6.0f} ms | inference median {row['inference_ms']['median']:6.0f} p90 {row['inference_ms']['p90']:6.0f} max {row['inference_ms']['max']:6.0f} ms | "
              f"total median {row['total_ms']['median']:6.0f} p90 {row['total_ms']['p90']:6.0f} ms | RAM {row['peak_working_set_mb']} MB | VRAM {row['peak_gpu_mib']} MiB", flush=True)
    (args.out / "winbench.json").write_text(json.dumps(report, indent=2))
    print(f"wrote {args.out / 'winbench.json'}")


if __name__ == "__main__":
    sys.exit(main())
