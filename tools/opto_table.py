"""The two-knob optical compressor's table (crates/chain/src/opto_table.rs) from the LA-2A model's probe results.

The model: a GRU trained on SignalTrain's LA-2A recordings (Compress and Limit, Peak Reduction 40-100), probed with
pink noise held at -44..0 dBFS (2 dB steps) and with level steps (stream-mic design notes comp-styles.md). Four
trained copies agreed within 1 dB from -40 to -10 dBFS and disagreed by 2-4 dB above it, so:
  - each curve is the gain reduction (dB) at -44..-10 dBFS, less what it shows at -44 (the quiet end, where it had
    not been trained), never falling, smoothed over three points;
  - above -10 dBFS it goes on with its own slope between -20 and -10 dBFS (at most 0.9 dB per dB);
  - the release (a fast and a slow stage, their shares) is a straight line over Peak Reduction fitted to the probed
    recovery after a 0.5 s hold, so one noisy setting does not make a step.
Usage: python3 tools/opto_table.py PROBE.json > crates/chain/src/opto_table.rs
"""
import json
import sys

import numpy as np
from scipy.optimize import least_squares

r = json.load(open(sys.argv[1]))
PRS = list(range(40, 101, 10))
LV = np.arange(-44, 1, 2)
TS = np.array([0.03, 0.06, 0.12, 0.25, 0.5, 1, 2, 5])
OUT_LV = np.arange(-60, 7, 2)   # the table's levels: -60..+6 dBFS


def curve(limit, pr):
    g = np.array(r[f"{limit}_{pr}"]["static"]["gr_mean"], float)
    g = g - g[0]
    g = np.maximum.accumulate(np.maximum(g, 0))
    g = np.convolve(np.pad(g, 1, mode="edge"), [1 / 3] * 3, "valid")
    i10, i20 = list(LV).index(-10), list(LV).index(-20)
    s = min(0.9, (g[i10] - g[i20]) / 10)
    out = []
    for x in OUT_LV:
        if x <= -44:
            out.append(0.0)
        elif x <= -10:
            out.append(float(np.interp(x, LV, g)))
        else:
            out.append(float(g[i10] + s * (x + 10)))
    return out


def release(limit, pr):
    rem = r[f"{limit}_{pr}"]["steps"]["0.5"]["remaining"]
    y = np.array([np.mean(rem["%g" % t]) for t in TS])
    f = least_squares(lambda p: p[0] * np.exp(-TS / p[1]) + (1 - p[0]) * np.exp(-TS / p[2]) - y, x0=[0.6, 0.05, 0.8],
                      bounds=([0, 0.005, 0.1], [1, 0.5, 10]))
    a, t1, t2 = f.x
    return t1 * 1000, t2 * 1000, (1 - a) * 100


print("// Made by tools/opto_table.py from the LA-2A model's probe results; see there. Do not edit by hand.")
print(f"/// the levels of each curve's points: {OUT_LV[0]} dBFS in steps of 2")
print(f"pub const LO: f64 = {OUT_LV[0]:.1f};")
print(f"pub const N: usize = {len(OUT_LV)};")
print(f"/// Peak Reduction of the rows: {PRS[0]}, {PRS[1]} .. {PRS[-1]}")
print(f"pub const PRS: [f64; {len(PRS)}] = [{', '.join(f'{p:.1f}' for p in PRS)}];")
for limit, name in ((0, "COMPRESS"), (1, "LIMIT")):
    print(f"/// gain reduction, dB, {name.lower()} mode")
    print(f"pub const {name}: [[f64; N]; {len(PRS)}] = [")
    for pr in PRS:
        print("    [" + ", ".join(f"{v:.2f}" for v in curve(limit, pr)) + "],")
    print("];")
    rel = np.array([release(limit, pr) for pr in PRS])
    pr = np.array(PRS, float)
    fit = [np.polyfit(pr, rel[:, j], 1) for j in range(3)]
    print(f"/// the release, {name.lower()} mode: fast ms, slow ms, slow share % = a + b * Peak Reduction")
    print(f"pub const {name}_REL: [[f64; 2]; 3] = [" + ", ".join(f"[{f[1]:.2f}, {f[0]:.4f}]" for f in fit) + "];")
