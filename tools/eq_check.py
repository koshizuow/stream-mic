"""Check the Rust EQ against an independent reference (build-plan.md step 1).

The reference builds each section's analog prototype (RBJ cookbook forms), prewarps the corner,
maps it with scipy's bilinear transform and runs it in float64. The Rust stage is Simper's SVF,
a different structure with the same response, so this checks both the design formulas and the
filter code.

Usage: uv run python eq_check.py CLI WORKDIR
"""
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy import signal

FS = 48000
LOW_HZ, PRES_HZ, PRES_Q, AIR_HZ = 200.0, 3000.0, 1.0, 10000.0
SHELF_Q = HP_Q = 1 / np.sqrt(2)

CASES = [
    dict(hp=80, low=0, pres=1.5, air=1),     # talk preset
    dict(hp=70, low=1, pres=1, air=1.5),     # singing preset
    dict(hp=160, low=-6, pres=6, air=-6),    # every knob at an end
    dict(hp=40, low=6, pres=-6, air=6),
    dict(hp=0, low=0, pres=0, air=0),        # flat
]


def analog(kind, gain_db, q):
    """Normalized prototype (corner at 1 rad/s): numerator, denominator in s."""
    A = 10 ** (gain_db / 40)
    if kind == "hp":
        return [1, 0, 0], [1, 1 / q, 1]
    if kind == "lowshelf":
        return [A, A * np.sqrt(A) / q, A * A], [A, np.sqrt(A) / q, 1]
    if kind == "highshelf":
        return [A * A, A * np.sqrt(A) / q, A], [1, np.sqrt(A) / q, A]
    if kind == "bell":
        return [1, A / q, 1], [1, 1 / (A * q), 1]
    raise ValueError(kind)


def digital(kind, f0, gain_db, q, fs=FS):
    b, a = analog(kind, gain_db, q)
    wc = 2 * fs * np.tan(np.pi * f0 / fs)  # prewarped corner, rad/s
    # Scale s -> s / wc: coefficient of s^k divided by wc^k.
    bs = [c / wc ** (2 - i) for i, c in enumerate(b)]
    as_ = [c / wc ** (2 - i) for i, c in enumerate(a)]
    return signal.bilinear(bs, as_, fs)


def sections(c):
    out = []
    if c["hp"] > 0:
        out.append(digital("hp", c["hp"], 0, HP_Q))
    out.append(digital("lowshelf", LOW_HZ, c["low"], SHELF_Q))
    out.append(digital("bell", PRES_HZ, c["pres"], PRES_Q))
    out.append(digital("highshelf", AIR_HZ, c["air"], SHELF_Q))
    return out


def reference(c, x):
    y = x.astype(np.float64)
    for b, a in sections(c):
        y = signal.lfilter(b, a, y)
    return y


def analog_db(c, f):
    """The unwarped analog response, to show the bilinear squeeze near Nyquist."""
    s = 1j * f
    h = np.ones_like(s)
    for kind, f0, g, q in [("hp", c["hp"], 0, HP_Q), ("lowshelf", LOW_HZ, c["low"], SHELF_Q),
                           ("bell", PRES_HZ, c["pres"], PRES_Q), ("highshelf", AIR_HZ, c["air"], SHELF_Q)]:
        if kind == "hp" and f0 <= 0:
            continue
        b, a = analog(kind, g, q)
        h *= np.polyval(b, s / f0) / np.polyval(a, s / f0)
    return 20 * np.log10(np.abs(h))


def test_signal():
    rng = np.random.default_rng(1)
    n = FS * 4
    x = rng.standard_normal(n) * 0.1
    t = np.arange(FS * 2) / FS
    sweep = 0.4 * signal.chirp(t, 20, t[-1], 23000, method="log")
    x[FS:FS * 3] += sweep
    x[FS * 3 + 100] += 0.9  # a click
    return x.astype(np.float32)


def run_cli(cli, inp, out, c, block):
    eq = ",".join(f"{k}={v}" for k, v in c.items())
    subprocess.run([cli, str(inp), str(out), "--block", str(block), "--eq", eq], check=True, capture_output=True)
    return sf.read(out, dtype="float64")[0]


def main():
    cli, work = sys.argv[1], Path(sys.argv[2])
    work.mkdir(parents=True, exist_ok=True)
    x = test_signal()
    inp = work / "eq_in.wav"
    sf.write(inp, x, FS, subtype="FLOAT")
    imp = np.zeros(FS, np.float32)
    imp[0] = 1
    sf.write(work / "eq_imp.wav", imp, FS, subtype="FLOAT")
    freqs = np.array([30, 60, 100, 200, 500, 1000, 3000, 6000, 10000, 14000, 18000, 22000])
    worst = -999.0
    ok = True
    for c in CASES:
        ref = reference(c, x)
        errs = []
        outs = {}
        for block in (1, 128, 4096):
            y = run_cli(cli, inp, work / f"eq_out_{block}.wav", c, block)
            outs[block] = y
            errs.append(20 * np.log10(np.sqrt(np.mean((y - ref) ** 2)) / np.sqrt(np.mean(ref ** 2))))
        same = all(np.array_equal(outs[1], outs[b]) for b in outs)
        e = max(errs)
        worst = max(worst, e)
        ok &= e < -100 and same
        # Frequency response of the Rust stage from its impulse response.
        h = run_cli(cli, work / "eq_imp.wav", work / "eq_imp_out.wav", c, 128)
        _, H = signal.freqz(h, worN=freqs, fs=FS)
        rust_db = 20 * np.log10(np.abs(H))
        ref_db = np.zeros(len(freqs))
        for b, a in sections(c):
            ref_db += 20 * np.log10(np.abs(signal.freqz(b, a, worN=freqs, fs=FS)[1]))
        an = analog_db(c, freqs.astype(float))
        resp_err = np.max(np.abs(rust_db - ref_db))
        ok &= resp_err < 0.001
        print(f"\n{c}: error vs reference {e:.1f} dB (block 1/128/4096 identical: {same}), response error {resp_err:.5f} dB")
        print("     Hz    rust  analog")
        for f, r, a in zip(freqs, rust_db, an):
            print(f"  {f:5d} {r:7.2f} {a:7.2f}")
    print(f"\nworst error {worst:.1f} dB: {'PASS' if ok else 'FAIL'}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
