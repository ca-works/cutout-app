#!/usr/bin/env python3
"""Convert BiRefNet (ZhengPeng7/BiRefNet, MIT) to a Core ML ML Program.

Usage (inside the pinned venv, see tools/convert/README.md):
    python -I tools/convert/convert.py --src build/convert/src --out build/convert/BiRefNet.mlpackage

What this does, and why:
* The upstream model code (`birefnet.py`) is loaded only after its SHA-256 is
  checked against SOURCES.txt. Weights come from `model.safetensors` (no
  pickle). `transformers` / `kornia` are replaced with tiny stubs: they are
  only used for the Hugging Face wrapper classes and a training-only branch.
* Core ML tensors are limited to rank 5. Swin's window_partition /
  window_reverse and BiRefNet's image2patches use rank-6 reshapes; they are
  replaced with equivalent rank-5 versions (batch size is fixed to 1).
* Core ML has no deformable convolution. Every `DeformableConv2d` (modulated
  deformable conv, torchvision `deform_conv2d`) is rewritten exactly as
      out = sum_k  W_k (1x1) * ( mask_k * bilinear_sample(x, p0 + p_k + offset_k) )
  using `grid_sample(align_corners=True, padding_mode="zeros")`, which has the
  same out-of-range behaviour as torchvision's kernel. Core ML maps
  grid_sample to `resample`.
* fp16 compute, static 1x3x1024x1024 input (ImageNet-normalised float, the
  same tensor the ONNX path feeds), single output: the final logits map
  passed through sigmoid (probabilities 0..1).
"""
import argparse
import hashlib
import importlib.util
import json
import sys
import time
import types
from pathlib import Path

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

HERE = Path(__file__).resolve().parent


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(8 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pinned_hashes():
    out = {}
    for line in (HERE / "SOURCES.txt").read_text().splitlines():
        parts = line.split()
        if len(parts) == 3 and not line.startswith("#"):
            out[parts[0]] = parts[2]
    return out


def verify_sources(src: Path):
    for name, digest in pinned_hashes().items():
        got = sha256(src / name)
        if got != digest:
            sys.exit(f"SHA-256 mismatch for {name}: {got} != {digest}")
    print("sources verified (SHA-256)")


# --------------------------------------------------------------- stubs

def install_stubs():
    tf = types.ModuleType("transformers")

    class PretrainedConfig:
        def __init__(self, **kwargs):
            for k, v in kwargs.items():
                setattr(self, k, v)

    class PreTrainedModel(nn.Module):
        def __init__(self, config=None, *a, **k):
            nn.Module.__init__(self)

    tf.PretrainedConfig = PretrainedConfig
    tf.PreTrainedModel = PreTrainedModel
    sys.modules["transformers"] = tf

    kornia = types.ModuleType("kornia")
    kf = types.ModuleType("kornia.filters")

    def laplacian(*a, **k):  # training-only branch
        raise RuntimeError("laplacian is training-only")

    kf.laplacian = laplacian
    kornia.filters = kf
    sys.modules["kornia"] = kornia
    sys.modules["kornia.filters"] = kf


def load_upstream(src: Path):
    install_stubs()
    pkg = types.ModuleType("birefnet_pkg")
    pkg.__path__ = [str(src)]
    sys.modules["birefnet_pkg"] = pkg
    for name in ("BiRefNet_config", "birefnet"):
        spec = importlib.util.spec_from_file_location(f"birefnet_pkg.{name}", src / f"{name}.py")
        mod = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = mod
        spec.loader.exec_module(mod)
    return sys.modules["birefnet_pkg.birefnet"]


# ------------------------------------------------- rank-5 replacements

def window_partition(x, window_size):
    B, H, W, C = x.shape
    assert B == 1
    x = x.reshape(H // window_size, window_size, W // window_size, window_size, C)
    return x.permute(0, 2, 1, 3, 4).reshape(-1, window_size, window_size, C)


def window_reverse(windows, window_size, H, W):
    C = windows.shape[-1]
    x = windows.reshape(H // window_size, W // window_size, window_size, window_size, C)
    return x.permute(0, 2, 1, 3, 4).reshape(1, H, W, C)


def make_image2patches(orig):
    def image2patches(image, grid_h=2, grid_w=2, patch_ref=None, transformation=None):
        if transformation != "b c (hg h) (wg w) -> b (c hg wg) h w":
            return orig(image, grid_h, grid_w, patch_ref, transformation)
        if patch_ref is not None:
            grid_h, grid_w = image.shape[-2] // patch_ref.shape[-2], image.shape[-1] // patch_ref.shape[-1]
        B, C, H, W = image.shape
        assert B == 1
        h, w = H // grid_h, W // grid_w
        x = image.reshape(C, grid_h, h, grid_w, w).permute(0, 1, 3, 2, 4)
        return x.reshape(1, C * grid_h * grid_w, h, w)
    return image2patches


# ---------------------------------------- deformable conv decomposition

def deform_forward_decomposed(self, x):
    """Exact modulated deformable conv via per-tap grid_sample."""
    offset = self.offset_conv(x)
    mask = 2.0 * torch.sigmoid(self.modulator_conv(x))
    weight = self.regular_conv.weight  # (Cout, Cin, kh, kw)
    kh, kw = weight.shape[2], weight.shape[3]
    pad = self.padding if isinstance(self.padding, tuple) else (self.padding, self.padding)
    assert self.stride == (1, 1)
    H, W = int(x.shape[2]), int(x.shape[3])  # static shapes: constants in the trace
    sy, sx = 2.0 / max(H - 1, 1), 2.0 / max(W - 1, 1)
    # base grid in normalised coordinates, folded with the tap position below
    ys = torch.tensor(np.arange(H, dtype=np.float32) * sy - 1.0).view(1, H, 1)
    xs = torch.tensor(np.arange(W, dtype=np.float32) * sx - 1.0).view(1, 1, W)
    out = None
    for ki in range(kh):
        for kj in range(kw):
            k = ki * kw + kj
            gy = ys + ((ki - pad[0]) * sy) + offset[:, 2 * k] * sy       # (1,H,W)
            gx = xs + ((kj - pad[1]) * sx) + offset[:, 2 * k + 1] * sx
            grid = torch.stack((gx, gy), dim=-1)  # (1,H,W,2)
            s = F.grid_sample(x, grid, mode="bilinear", padding_mode="zeros", align_corners=True)
            s = s * mask[:, k:k + 1]
            y = F.conv2d(s, weight[:, :, ki:ki + 1, kj:kj + 1])
            out = y if out is None else out + y
    if self.regular_conv.bias is not None:
        out = out + self.regular_conv.bias.view(1, -1, 1, 1)
    return out


class Wrapped(nn.Module):
    def __init__(self, net):
        super().__init__()
        self.net = net

    def forward(self, x):
        return torch.sigmoid(self.net(x)[-1])


def build(src: Path, decompose: bool):
    m = load_upstream(src)
    m.window_partition = window_partition
    m.window_reverse = window_reverse
    m.image2patches = make_image2patches(m.image2patches)
    net = m.BiRefNet(bb_pretrained=False)
    from safetensors.torch import load_file
    sd = load_file(str(src / "model.safetensors"))
    missing, unexpected = net.load_state_dict(sd, strict=False)
    if missing or unexpected:
        sys.exit(f"state dict mismatch: missing={missing[:5]} unexpected={unexpected[:5]}")
    net.eval().float()
    n = 0
    if decompose:
        for mod in net.modules():
            if type(mod).__name__ == "DeformableConv2d":
                mod.forward = types.MethodType(deform_forward_decomposed, mod)
                n += 1
    return Wrapped(net).eval(), n


def test_input(path):
    if path is None:
        g = torch.Generator().manual_seed(0)
        return torch.randn(1, 3, 1024, 1024, generator=g)
    from PIL import Image
    img = Image.open(path).convert("RGB").resize((1024, 1024), Image.BILINEAR)
    a = np.asarray(img).astype(np.float32) / 255.0
    a = (a - np.array([0.485, 0.456, 0.406], np.float32)) / np.array([0.229, 0.224, 0.225], np.float32)
    return torch.from_numpy(a.transpose(2, 0, 1)[None].copy())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", type=Path, default=Path("build/convert/src"))
    ap.add_argument("--out", type=Path, default=Path("build/convert/BiRefNet.mlpackage"))
    ap.add_argument("--image", type=Path, help="image for the PyTorch decomposition check")
    ap.add_argument("--precision", choices=["fp16", "fp32"], default="fp16")
    ap.add_argument("--skip-check", action="store_true")
    args = ap.parse_args()

    verify_sources(args.src)
    torch.set_grad_enabled(False)
    x = test_input(args.image)

    model, n = build(args.src, decompose=True)
    print(f"replaced {n} deformable convolutions")
    if not args.skip_check:
        ref, _ = build(args.src, decompose=False)
        t = time.time(); a = ref(x); print(f"torch reference (deform_conv2d) {time.time()-t:.1f}s")
        t = time.time(); b = model(x); print(f"torch decomposed {time.time()-t:.1f}s")
        d = (a - b).abs()
        print(f"decomposition max|Δp| {d.max().item():.2e}  mean {d.mean().item():.2e}")
        if d.max().item() > 1e-3:
            sys.exit("decomposition does not match deform_conv2d")
        del ref

    import coremltools as ct
    t = time.time()
    traced = torch.jit.trace(model, x, check_trace=False)
    print(f"traced in {time.time()-t:.1f}s")
    t = time.time()
    ml = ct.convert(
        traced,
        convert_to="mlprogram",
        inputs=[ct.TensorType(name="input", shape=(1, 3, 1024, 1024), dtype=np.float32)],
        outputs=[ct.TensorType(name="alpha", dtype=np.float32)],
        compute_precision=ct.precision.FLOAT16 if args.precision == "fp16" else ct.precision.FLOAT32,
        minimum_deployment_target=ct.target.macOS14,
        compute_units=ct.ComputeUnit.ALL,
        skip_model_load=True,  # device compilation happens in the app / bench, not here
    )
    print(f"converted in {time.time()-t:.1f}s")
    ml.short_description = "BiRefNet (general, ZhengPeng7, MIT) - dichotomous image segmentation, 1024x1024"
    ml.author = "ZhengPeng7 (model); CutOut conversion recipe tools/convert"
    ml.license = "MIT"
    ml.user_defined_metadata["source"] = "huggingface.co/ZhengPeng7/BiRefNet@66f7f80c1a64b3c4df327a07eeca4e87130b787b"
    ml.user_defined_metadata["recipe"] = "deform-conv->grid_sample decomposition; rank5 window partition; " + args.precision
    args.out.parent.mkdir(parents=True, exist_ok=True)
    ml.save(str(args.out))
    w = args.out / "Data/com.apple.CoreML/weights/weight.bin"
    print(json.dumps({"out": str(args.out), "weight_bin_sha256": sha256(w), "weight_bin_bytes": w.stat().st_size}))


if __name__ == "__main__":
    main()
