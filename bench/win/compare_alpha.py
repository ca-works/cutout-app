import sys, glob, os, json
import numpy as np
from PIL import Image
root = r"C:\dev\bench\final"
pairs = {"gpu(v2) vs legacy fp16-gpu": ("gpu","legacy-gpu"), "cpu(v2) vs legacy fp32-cpu": ("cpu","legacy-cpu"), "gpu(v2) vs cpu(v2)": ("gpu","cpu"), "legacy-gpu vs legacy-cpu": ("legacy-gpu","legacy-cpu")}
def load(cfg, name):
    f = glob.glob(os.path.join(root, cfg, name + "__*.png"))[0]
    return np.asarray(Image.open(f).convert("RGBA"))[..., 3].astype(np.float32) / 255
names = sorted(os.path.basename(f).split("__")[0] for f in glob.glob(os.path.join(root, "gpu", "*.png")))
out = {}
for label, (a, b) in pairs.items():
    print("==", label)
    for n in names:
        A, B = load(a, n), load(b, n)
        mad = float(np.abs(A - B).mean())
        ba, bb = A > .5, B > .5
        iou = float((ba & bb).sum() / max(1, (ba | bb).sum()))
        mx = float(np.abs(A - B).max())
        print(f"  {n:12s} MAD {mad:.5f}  IoU {iou:.5f}  max|d| {mx:.3f}")
        out.setdefault(label, {})[n] = {"mad": mad, "iou": iou, "max": mx}
json.dump(out, open(os.path.join(root, "cmp", "alpha_compare.json"), "w"), indent=1)
