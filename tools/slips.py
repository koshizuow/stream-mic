# /// script
# dependencies = ["numpy", "soundfile"]
# ///
"""Do two recordings of the same node match sample for sample (16-bit tolerance)? Follows the offset of A in B in
0.1 s windows: a window that matches at the current offset is fine; otherwise it is searched within +-2 s, and a new
offset counts once it holds for 1 s (music that repeats matches at several offsets for a moment). Prints the slips
and the windows that match nowhere. Usage: uv run tools/slips.py A.wav B.wav"""
import sys

import numpy as np
import soundfile as sf

a = sf.read(sys.argv[1], dtype="float64", always_2d=True)[0][:, 0]
b = sf.read(sys.argv[2], dtype="float64", always_2d=True)[0][:, 0]
R = 48000
W = R // 10
TOL = 1e-4


def same(t, k):
    return 0 <= t + k and t + k + W <= len(b) and np.max(np.abs(a[t:t + W] - b[t + k:t + k + W])) < TOL


def matches(t, centre, span):
    x = a[t:t + W]
    lo, hi = max(0, t + centre - span), min(len(b), t + centre + W + span)
    seg = b[lo:hi]
    if len(seg) < W:
        return []
    m = 1 << int(np.ceil(np.log2(len(seg) + W)))
    c = np.fft.irfft(np.fft.rfft(seg, m) * np.conj(np.fft.rfft(x, m)), m)[:len(seg) - W + 1]
    return [lo + j - t for j in np.argsort(c)[::-1][:12] if np.max(np.abs(seg[j:j + W] - x)) < TOL]


loud = lambda t: np.max(np.abs(a[t:t + W])) >= 0.003
t, ms = 0, []
while not ms and t + W < len(a):
    if np.max(np.abs(a[t:t + W])) >= 0.01:
        ms = matches(t, 0, 5 * R)
    t += W
if not ms:
    sys.exit("no window of A found in B")
k = ms[0]
print(f"{sys.argv[1]} vs {sys.argv[2]}: offset {k} samples ({k / 48:.2f} ms)")
cand, held, differ, checked, slips = None, 0, [], 0, 0
for t in range(0, len(a) - W, W):
    if not loud(t) or t + k < 0 or t + k + W > len(b):
        continue
    checked += 1
    if same(t, k):
        cand, held = None, 0
        continue
    ms = matches(t, k, 2 * R)
    if not ms:
        differ.append(t / R)
        continue
    j = min(ms, key=lambda x: abs(x - k))
    cand, held = (j, held + 1) if j == cand else (j, 1)
    if held >= 10:
        print(f"  slip at {(t - 9 * W) / R:7.2f}s: {j - k:+d} samples ({(j - k) / 48:+.2f} ms)")
        k, cand, held, slips = j, None, 0, slips + 1
print(f"  {checked} windows with sound, {slips} slips, {len(differ)} match nowhere" + (f": {differ[:10]}" if differ else ""))
