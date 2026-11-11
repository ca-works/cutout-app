# Windows performance and quality (v2 model package)

Measured on **2026-11-11** on the development PC: Windows 11 Pro 26200, NVIDIA GeForce RTX 4070 Laptop
GPU (8 GB) + AMD Radeon 780M (integrated), release build of the code at commit `b2d42c9`/`1dfc649`.
Everything below was measured here unless it is marked *previously reported*. Other hardware will differ;
only this one PC (and its two GPUs) was available.

## How to reproduce

```powershell
cargo build --release -p cutout-cli --features directml
python bench/win/winbench.py --cli <cutout-cli.exe> --models <dir with the model files> --images <dir> --out <dir> --runs 2
python bench/win/compare_alpha.py            # alpha comparison between configurations (edit the paths at the top)
```

`winbench.py` runs `cutout-cli bench` per configuration (v2 GPU/CPU and the previous onnx-community fp16/fp32
exports as the baseline) and records load time, inference/pre/post time per image, peak working set and the
peak video memory of the NVIDIA card (`nvidia-smi`, whole card minus its use before the run, so it includes
anything else the card does at that moment). Raw results: `bench/win/results-2026-11-11.json`.

## Model package

| | Previous (RC2) | v2 |
|---|---|---|
| Files | `birefnet-general-fp32.onnx` + `-fp16.onnx` | `birefnet-general-v2-cpu.onnx`, `-gpu.onnx`, `.weights` (fp16 weights shared) |
| Size | 1,462 MB | 588 MB |
| Installer | 1,476,234,388 bytes (1.37 GiB) | **601,682,891 bytes (574 MiB)** |

## Background removal, six photographs (1–8 MP), 2 runs each, warm session

| Configuration | Model load | Inference (median / max) | Total incl. post (median) | Peak RAM (working set) | Peak VRAM |
|---|---|---|---|---|---|
| **v2 GPU (DirectML, RTX 4070)** | 2.8 s | **0.32 s** / 0.33 s | **0.43 s** | **0.86 GB** | 5.3 GB |
| previous fp16 GPU | 6.7 s | 0.50 s / 0.51 s | 0.62 s | 0.96 GB | 7.1 GB |
| **v2 CPU** | 2.4 s | **6.9 s** / 7.2 s | **7.1 s** | **3.4 GB** | – |
| previous fp32 CPU | 6.4 s | 10.6 s / 11.0 s | 10.8 s | 7.2 GB | – |

Notes:

* Memory: the GPU process needs only **0.86 GB of host RAM**; the video-memory peak is **5.3 GB** (7.1 GB for
  the old fp16 graph), which is why the app only uses the GPU on an adapter with at least 5.5 GB of dedicated
  memory (`app/src/platform/gpu.rs`).
* Installed app, scripted removal (`removal.json`, 640x480, includes start-up, open, removal, export, quit):
  GPU 5 s wall, 0.67 GB peak; CPU 11 s wall, 3.3 GB peak. In-app timing (`CUTOUT_TRACE_TIMING=1`): GPU session
  pre-loaded in the background in 2.8 s after the image is opened, then inference 1.07 s for the first
  prediction of the session (0.32 s steady state in the CLI benchmark).
* Cold CPU: verify 0.3 s + load 2.1 s + inference 6.9 s.

## High resolution (GPU, 2 runs)

| Image | Pre-processing | Inference | Post-processing | Total |
|---|---|---|---|---|
| 36 MP (4800x7500) | 19 ms | 300 ms | 351 ms | 0.67 s |
| 60 MP (6900x8700) | 28 ms | 309 ms | 514 ms | 0.85 s |

*Previously reported baselines (not re-measured): 36 MP pre 217 ms / post 1.90 s; 60 MP pre 504 ms, whole
60 MP pipeline 11.9 s.* The current numbers agree with the reported improved values (20 ms, 33 ms, 0.34 s).

## Clipboard, export, start-up

| Item | Result |
|---|---|
| Clipboard PNG encode, 36 MP | 233 ms (file-quality PNG: 3,416 ms) |
| Clipboard PNG encode, 8 MP | 49 ms (file-quality PNG: 557 ms) |
| JPEG encode, 36 MP | 649 ms |
| Decode 36 MP JPEG | 122 ms |
| Start-up to first frames (installed app, 8 launches) | **122–133 ms** |

Export runs on a worker thread (the UI stays responsive); this is exercised by the smoke script.

## Output quality (the new models against the old ones)

The alpha mattes of the six photographs (hair, fur, a dandelion, a wine glass, a black mug next to a white
cup, a bicycle) were compared pixel by pixel (`bench/win/alpha-compare-2026-11-11.json`). MAD is the mean
absolute alpha difference (0–1), IoU is computed on the mattes binarised at 0.5.

| Pair | MAD | IoU |
|---|---|---|
| v2 GPU vs previous fp16 GPU | 0.0002–0.0012 | 0.992–0.9998 |
| v2 CPU vs previous fp32 CPU | 0.0001–0.0011 | 0.992–0.9998 |
| v2 GPU vs v2 CPU | 0.0001–0.0003 | 0.9985–0.9999 |
| previous GPU vs previous CPU (reference spread) | 0.0001–0.0004 | 0.997–0.9999 |

The lowest IoU is the dandelion (0.992, MAD 0.0008): its translucent seed head differs by a few per cent in
alpha, which is the same order as the difference between the old GPU and CPU graphs. Visual side-by-side
(photo | previous model | v2): [docs/images/windows-v2-vs-legacy.jpg](images/windows-v2-vs-legacy.jpg); no
difference is visible in hair, fur, glass or the bicycle spokes. The synthetic-scene parity check of
`bench/golden` and the unit tests (34 core + 10 app, plus 2 real-clipboard tests) pass on the final code.

## Hardware selection (what was and was not tested)

* The GPU is used only when the adapter Windows ranks as high-performance is a real (non-software) adapter with
  at least 5.5 GB of dedicated memory. Otherwise, or when the GPU session fails once, the CPU graph runs.
* Tested here: RTX 4070 Laptop (8 GB) -> GPU path; Radeon 780M (integrated, shared memory) -> not selected,
  CPU path (an earlier experiment with the iGPU either finished in 3.6 s or hit the driver time-out).
  Simulated GPU failure and a missing `DirectML.dll` both fall back to the CPU (tests above).
* **Not tested:** cards with 6 GB or less, other NVIDIA/AMD/Intel discrete GPUs, Windows 10, ARM64.
  The 5.5 GB limit is conservative for that reason.
