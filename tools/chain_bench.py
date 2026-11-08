"""Stacked CPU and latency of the whole chain, stage by stage (user 2026-11-06: measure them added up).

  taskset -c 2 uv run python chain_bench.py IN.wav OUT_DIR [BLOCK] [DF_MIN_BUFFER_FRAMES]

Blocks are fed at real-time pace (the DeepFilterNet plugin hands frames to its own thread and underruns when fed
faster). Stage CPU is the time spent in the plugin's run() call; whatever other threads use (DeepFilterNet's worker) is
the process CPU minus the main thread's, added to the denoise row.

The chain runs block by block (128 samples by default, a common PipeWire quantum) in one process, every stage on
the same core, as the real-time engine will. For each stage it reports:
- its own CPU (run() time / audio time) and the cumulative CPU up to it
- the cumulative 99th percentile and worst block time against the block period (the real-time deadline)
- the cumulative latency: the output tapped after this stage, aligned to the input by cross-correlation

The plugins are stand-ins for the method chosen for each stage (DeepFilterNet's LADSPA build with its low-latency
model, Calf and Dragonfly for the rest). Our own Rust stages replace them later and the same table is filled again.
Block buffering (one block in, one out) is not in the latency column; add 2 x BLOCK / fs.
"""
import json
import resource
import sys
import time
from pathlib import Path

import numpy as np
import soundfile as sf

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
from ladspa import LadspaPlugin  # noqa: E402
from lv2host import LV2Plugin  # noqa: E402

DF_LIB = str(Path.home() / "denoise/df/libdeep_filter_ladspa-0.5.6-x86_64-unknown-linux-gnu.so")

DF_BUF = [None]

STAGES = [
    ("denoise (DF3 ll)", lambda fs, b: LadspaPlugin(DF_LIB, "deep_filter_mono", fs, dict(
        {"Attenuation Limit": 100}, **({"Min Processing Buffer": DF_BUF[0]} if DF_BUF[0] is not None else {})), b)),
    ("low cut + EQ", lambda fs, b: LV2Plugin("http://calf.sourceforge.net/plugins/Equalizer8Band", fs,
                                             {"hp_active": 1, "hp_freq": 80, "p1_active": 1, "p1_level": 1.4,
                                              "p1_freq": 3000, "hs_active": 1, "hs_level": 1.4, "hs_freq": 10000}, b)),
    ("de-esser", lambda fs, b: LV2Plugin("http://calf.sourceforge.net/plugins/Deesser", fs, {"mode": 1}, b)),
    ("expander", lambda fs, b: LV2Plugin("http://calf.sourceforge.net/plugins/Gate", fs,
                                         {"threshold": 0.01, "ratio": 2, "range": 0.125}, b)),
    ("compressor", lambda fs, b: LV2Plugin("http://calf.sourceforge.net/plugins/MonoCompressor", fs,
                                           {"threshold": 0.125, "ratio": 3, "attack": 10, "release": 120, "knee": 2.8}, b)),
    ("limiter", lambda fs, b: LV2Plugin("http://calf.sourceforge.net/plugins/Limiter", fs,
                                        {"limit": 0.891, "attack": 2, "release": 50, "auto_level": 0}, b)),
    ("reverb (plate)", lambda fs, b: LV2Plugin("urn:dragonfly:plate", fs, {"dry_level": 100, "early_level": 15}, b)),
]


def lag_ms(x, y, fs, max_ms=200):
    n = min(len(x), len(y), fs * 10)
    a, b = x[:n], y[:n]
    m = int(max_ms * 1e-3 * fs)
    c = np.fft.irfft(np.fft.rfft(b, 2 * n) * np.conj(np.fft.rfft(a, 2 * n)))[: m + 1]
    return int(np.argmax(c)) / fs * 1e3


def main():
    src, out = sys.argv[1], Path(sys.argv[2])
    block = int(sys.argv[3]) if len(sys.argv) > 3 else 128
    if len(sys.argv) > 4:
        DF_BUF[0] = float(sys.argv[4])
    out.mkdir(parents=True, exist_ok=True)
    x, fs = sf.read(src, dtype="float32")
    x = x if x.ndim == 1 else x[:, 0]
    plugs = [(name, make(fs, block)) for name, make in STAGES]
    nb = len(x) // block
    times = np.zeros((nb, len(plugs)))
    taps = [np.zeros(nb * block, np.float32) for _ in plugs]
    period = block / fs
    ru0, th0 = resource.getrusage(resource.RUSAGE_SELF), time.thread_time()
    start = time.perf_counter()
    for i in range(nb):
        buf = x[i * block:(i + 1) * block, None]
        for k, (_, p) in enumerate(plugs):
            tm = [0.0]
            y = p.run(buf, tm)  # times only the plugin's own run(), not the Python glue around it
            times[i, k] = tm[0]
            buf = y[:, :1] if y.shape[1] == 1 else y.mean(1, keepdims=True)
            taps[k][i * block:(i + 1) * block] = buf[:, 0]
        wait = start + (i + 1) * period - time.perf_counter()
        if wait > 0:
            time.sleep(wait)
    ru1, th1 = resource.getrusage(resource.RUSAGE_SELF), time.thread_time()
    proc = (ru1.ru_utime + ru1.ru_stime) - (ru0.ru_utime + ru0.ru_stime)
    other = max(0.0, proc - (th1 - th0))
    for _, p in plugs:
        p.close()
    dur = nb * block / fs
    cum = np.cumsum(times, 1)
    stage_cpu = times.sum(0)
    stage_cpu[0] += other
    cum_cpu = np.cumsum(stage_cpu)
    print(f"other threads (DeepFilterNet worker): {100 * other / dur:.2f} % of one core")
    rows = []
    print(f"{nb} blocks of {block} ({period * 1e3:.2f} ms), {dur:.1f} s audio, one core")
    print(f"{'stage':18} {'own %':>6} {'cum %':>6} {'main p99 ms':>10} {'main max ms':>10} {'cum lat ms':>10}")
    for k, (name, p) in enumerate(plugs):
        r = {"stage": name, "own_pct": 100 * stage_cpu[k] / dur, "cum_pct": 100 * cum_cpu[k] / dur,
             "cum_p99_ms": float(np.percentile(cum[:, k], 99) * 1e3), "cum_max_ms": float(cum[:, k].max() * 1e3),
             "cum_lat_ms": lag_ms(x, taps[k], fs)}
        rows.append(r)
        sf.write(out / f"tap{k}.wav", taps[k], fs, subtype="FLOAT")
        print(f"{name:18} {r['own_pct']:6.2f} {r['cum_pct']:6.2f} {r['cum_p99_ms']:10.3f} {r['cum_max_ms']:10.3f} "
              f"{r['cum_lat_ms']:10.2f}")
    print(f"deadline per block {period * 1e3:.2f} ms; buffering adds {2 * period * 1e3:.2f} ms latency")
    (out / "chain.json").write_text(json.dumps({"block": block, "fs": fs, "rows": rows}, indent=1))


if __name__ == "__main__":
    main()
