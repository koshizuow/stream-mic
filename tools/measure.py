"""Measure a processor from its input and output test signals (made by signals.py).

Gain is compared as a trajectory in dB, not as a waveform (Thompson and Heilemann 2026 show waveform
losses barely separate different gain behaviour).

  static IN OUT          ramp: output level vs input level, prints threshold-ish points and slope above
  gain IN OUT            bursts: gain trajectory, attack and release times (63 % of the change)
  response IN OUT        noise: magnitude response from the cross spectrum
  decay OUT              impulse response: T60 by Schroeder backward integration (EDT, T20, T30)
  thd IN OUT             lowsine: harmonic distortion of the output

Add --plot PNG to save a figure. Plugins may add latency, so OUT is aligned to IN first.
"""
import argparse

import numpy as np
import soundfile as sf
from scipy import signal


def load(path):
    x, fs = sf.read(path, dtype="float64")
    if x.ndim > 1:
        x = x[:, 0]
    return x, fs


def align(x, y, fs, max_ms=50):
    """Delay y so it lines up with x (plugins with lookahead report latency)."""
    n = min(len(x), len(y), fs * 5)
    lag_max = int(max_ms * 1e-3 * fs)
    seg_x = x[:n]
    seg_y = y[:n]
    corr = signal.correlate(seg_y, seg_x, mode="full", method="fft")
    mid = len(seg_x) - 1
    window = corr[mid:mid + lag_max + 1]
    lag = int(np.argmax(np.abs(window)))
    y = y[lag:]
    m = min(len(x), len(y))
    return x[:m], y[:m], lag


def envelope_db(x, fs, win_ms=10.0):
    """RMS level in dB over a sliding window (one period of 1 kHz is 1 ms, so 10 ms is smooth)."""
    n = max(1, int(win_ms * 1e-3 * fs))
    p = signal.fftconvolve(x ** 2, np.ones(n) / n, mode="same")
    return 10 * np.log10(np.maximum(p, 1e-20)) + 3.01  # sine RMS to peak level


def static(args):
    x, fs = load(args.inp)
    y, _ = load(args.out)
    x, y, lag = align(x, y, fs)
    lx, ly = envelope_db(x, fs, 20), envelope_db(y, fs, 20)
    mask = lx > -70
    lx, ly = lx[mask], ly[mask]
    bins = np.arange(-60, 0.5, 1.0)
    idx = np.digitize(lx, bins)
    curve = [(float(np.median(lx[idx == i])), float(np.median(ly[idx == i])))
             for i in range(1, len(bins)) if np.any(idx == i)]
    xs = np.array([c[0] for c in curve])
    ys = np.array([c[1] for c in curve])
    gr = ys - xs
    print(f"latency {lag} samples ({lag / fs * 1e3:.2f} ms)")
    print("in_dB  out_dB  gain_dB")
    for a, b, g in zip(xs, ys, gr):
        if int(a) % 4 == 0 or args.what == "static" and a < -30:
            print(f"{a:6.0f} {b:7.2f} {g:7.2f}")
    top = xs >= xs.max() - 10
    slope = np.polyfit(xs[top], ys[top], 1)[0]
    print(f"slope over the top 10 dB {slope:.3f} (ratio {1 / slope if slope > 0 else float('inf'):.2f}:1)")
    onset = xs[np.argmax(gr < gr[0] - 0.5)] if np.any(gr < gr[0] - 0.5) else None
    print(f"gain falls 0.5 dB below its low-level value at input {onset} dB")
    if args.plot:
        import matplotlib.pyplot as plt
        fig, ax = plt.subplots(1, 2, figsize=(10, 4))
        ax[0].plot(xs, ys); ax[0].plot(xs, xs, "--", lw=0.8)
        ax[0].set_xlabel("input dB"); ax[0].set_ylabel("output dB"); ax[0].grid(True)
        ax[1].plot(xs, gr); ax[1].set_xlabel("input dB"); ax[1].set_ylabel("gain dB"); ax[1].grid(True)
        fig.tight_layout(); fig.savefig(args.plot, dpi=110)
    return xs, ys


def step_time(t, g, start, end, frac=0.632):
    """Time from t[start] for g to cover frac of its change between start and end."""
    g0, g1 = g[start], g[end]
    target = g0 + frac * (g1 - g0)
    seg = g[start:end]
    hit = np.nonzero((seg - target) * np.sign(g1 - g0) >= 0)[0]
    return (t[start + hit[0]] - t[start]) * 1e3 if len(hit) else float("nan")


def gain(args):
    x, fs = load(args.inp)
    y, _ = load(args.out)
    x, y, lag = align(x, y, fs)
    lx, ly = envelope_db(x, fs, 2), envelope_db(y, fs, 2)
    g = ly - lx
    t = np.arange(len(g)) / fs
    hi = lx > -25
    edges = np.flatnonzero(np.diff(hi.astype(int)))
    print(f"latency {lag} samples ({lag / fs * 1e3:.2f} ms)")
    for k in range(0, len(edges) - 1, 2):
        up, down = edges[k], edges[k + 1]
        length = (down - up) / fs
        settle = min(down - 1, up + int(0.9 * (down - up)))
        nxt = edges[k + 2] if k + 2 < len(edges) else len(g) - 1
        rest = min(nxt - 1, down + int(2.9 * fs))
        a = step_time(t, g, up + int(0.003 * fs), settle) if length > 0.01 else float("nan")
        r = step_time(t, g, down + int(0.003 * fs), rest)
        print(f"burst {length * 1e3:6.0f} ms: gain at end {g[down - int(0.003 * fs)]:6.2f} dB, "
              f"attack(63%) {a:7.1f} ms, release(63%) {r:7.1f} ms, gain 2.5 s after {g[min(len(g)-1, down + int(2.5 * fs))]:6.2f} dB")
    if args.plot:
        import matplotlib.pyplot as plt
        fig, ax = plt.subplots(figsize=(11, 4))
        ax.plot(t, g, lw=0.8); ax.set_xlabel("s"); ax.set_ylabel("gain dB"); ax.grid(True)
        fig.tight_layout(); fig.savefig(args.plot, dpi=110)


def gate(args):
    """Gates and expanders: the static curve under the threshold, and how fast the gate opens and closes, from the gain
    trajectory with a 1 ms window (open: burst start until within 1 dB of fully open; close: burst end until half and
    90 % of the final reduction in dB)."""
    x, fs = load(args.inp)
    y, _ = load(args.out)
    x, y, lag = align(x, y, fs)
    lx, ly = envelope_db(x, fs, 1), envelope_db(y, fs, 1)
    g = ly - lx
    hi = lx > -25
    edges = np.flatnonzero(np.diff(hi.astype(int)))
    floor = float(np.median(g[int(0.5 * fs):int(0.9 * fs)]))
    print(f"latency {lag} samples, reduction while closed {floor:.2f} dB")
    for k in range(0, len(edges) - 1, 2):
        up, down = edges[k] + 1, edges[k + 1] + 1
        seg = g[up:down]
        op = np.flatnonzero(seg > -1.0)
        nxt = edges[k + 2] if k + 2 < len(edges) else len(g) - 1
        tail = g[down:nxt]
        half = np.flatnonzero(tail < floor / 2)
        ninety = np.flatnonzero(tail < floor * 0.9)
        f = lambda a: f"{a[0] / fs * 1e3:7.1f}" if len(a) else "    n/a"
        print(f"burst {(down - up) / fs * 1e3:6.0f} ms: open {f(op)} ms, close to half {f(half)} ms, to 90% {f(ninety)} ms")


def response(args):
    x, fs = load(args.inp)
    y, _ = load(args.out)
    x, y, lag = align(x, y, fs)
    f, pxy = signal.csd(x, y, fs, nperseg=8192)
    _, pxx = signal.welch(x, fs, nperseg=8192)
    h = 20 * np.log10(np.abs(pxy / pxx) + 1e-12)
    print(f"latency {lag} samples ({lag / fs * 1e3:.2f} ms)")
    for fc in (50, 100, 200, 500, 1000, 2000, 4000, 6000, 8000, 10000, 12000, 16000, 20000):
        i = np.argmin(np.abs(f - fc))
        print(f"{fc:6d} Hz {h[i]:7.2f} dB")
    if args.plot:
        import matplotlib.pyplot as plt
        fig, ax = plt.subplots(figsize=(9, 4))
        ax.semilogx(f[1:], h[1:]); ax.set_xlim(20, fs / 2); ax.set_xlabel("Hz"); ax.set_ylabel("dB"); ax.grid(True, which="both")
        fig.tight_layout(); fig.savefig(args.plot, dpi=110)


def echo_density(h, fs, win_ms=20.0):
    """Normalized echo density (Abel and Huang 2006): share of samples beyond one standard deviation
    in a sliding window, divided by the Gaussian value erfc(1/sqrt 2) = 0.3173. Reaches about 1 when
    the response sounds like noise (fully diffuse); low values early on are audible as separate echoes."""
    n = int(win_ms * 1e-3 * fs)
    out = []
    for i in range(0, len(h) - n, n // 4):
        w = h[i:i + n]
        sd = np.sqrt(np.mean(w ** 2))
        out.append((i + n / 2) / fs * 1e3 and np.mean(np.abs(w) > sd) / 0.3173)
    return np.array(out), n // 4


def decay(args):
    full, fs = sf.read(args.out, dtype="float64")
    if full.ndim > 1 and full.shape[1] > 1:
        l, r = full[:, 0], full[:, 1]
        s0 = int(np.argmax((np.abs(l) + np.abs(r)) > 1e-6))
        a, b = l[s0:s0 + fs], r[s0:s0 + fs]
        corr = np.sum(a * b) / np.sqrt(np.sum(a * a) * np.sum(b * b) + 1e-30)
        print(f"left-right correlation over the first second {corr:.3f}")
    y, fs = load(args.out)
    start = int(np.argmax(np.abs(y) > 1e-6))
    ned, hop = echo_density(y[start:], fs)
    first = np.argmax(ned > 0.9) if np.any(ned > 0.9) else None
    early = ned[: int(0.05 * fs / hop)]
    print(f"echo density: mean over the first 50 ms {early.mean():.2f}, reaches 0.9 at "
          f"{(first * hop / fs * 1e3) if first is not None else float('nan'):.0f} ms")
    h = y[start:]
    edc = np.cumsum(h[::-1] ** 2)[::-1]
    edc_db = 10 * np.log10(edc / edc[0] + 1e-30)
    t = np.arange(len(h)) / fs
    def fit(lo, hi):
        sel = (edc_db <= lo) & (edc_db >= hi)
        if sel.sum() < 10:
            return float("nan")
        slope = np.polyfit(t[sel], edc_db[sel], 1)[0]
        return -60 / slope
    print(f"onset {start / fs * 1e3:.1f} ms after the file start")
    print(f"EDT {fit(0, -10):.2f} s  T20 {fit(-5, -25):.2f} s  T30 {fit(-5, -35):.2f} s")
    if args.plot:
        import matplotlib.pyplot as plt
        fig, ax = plt.subplots(figsize=(9, 4))
        ax.plot(t, edc_db); ax.set_ylim(-80, 2); ax.set_xlabel("s"); ax.set_ylabel("energy decay dB"); ax.grid(True)
        fig.tight_layout(); fig.savefig(args.plot, dpi=110)


def thd(args):
    x, fs = load(args.inp)
    y, _ = load(args.out)
    seg = y[len(y) // 4: len(y) // 4 + fs]
    spec = np.abs(np.fft.rfft(seg * np.hanning(len(seg))))
    f = np.fft.rfftfreq(len(seg), 1 / fs)
    f0 = f[np.argmax(spec)]
    def amp(fr):
        i = np.argmin(np.abs(f - fr))
        return spec[max(0, i - 3): i + 4].max()
    fund = amp(f0)
    harm = np.sqrt(sum(amp(k * f0) ** 2 for k in range(2, 11) if k * f0 < fs / 2))
    print(f"fundamental {f0:.0f} Hz, THD {100 * harm / fund:.3f} %")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("what", choices=["static", "gain", "gate", "response", "decay", "thd"])
    p.add_argument("files", nargs="+")
    p.add_argument("--plot")
    a = p.parse_args()
    if a.what == "decay":
        a.out = a.files[0]
    else:
        a.inp, a.out = a.files[0], a.files[1]
    {"static": static, "gain": gain, "gate": gate, "response": response, "decay": decay, "thd": thd}[a.what](a)


if __name__ == "__main__":
    main()
