"""Blind listening set for the whole chain (build-plan.md step 5): our Rust chain against the plugin chain
(chain_bench.py's stand-ins) set as close to the same preset as each plugin allows, the outputs matched in level.

Speech: real iPad recordings (DAPS device, omni mic in a room) through talk-omni. Singing: VocalSet with real room
tone at 30 dB SNR through sing, the reverb off on both (it has its own page, stream-mic-verb), so this one is about
the denoise, EQ, dynamics and limiter.

The plugin chain is fed at real-time pace (DeepFilterNet's plugin underruns when fed faster) and has the plugin's
30 ms latency; both outputs are aligned to the input before the level match.

Usage: taskset -c 2 uv run python chain_listen.py CLI OUT_DIR DATA_DIR TONE_DIR
"""
import html
import random
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import soundfile as sf

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
from chain_bench import DF_LIB, lag_ms  # noqa: E402
from ladspa import LadspaPlugin  # noqa: E402
from lv2host import LV2Plugin  # noqa: E402

FS = 48000
BLOCK = 128


def db(v):
    return 10 ** (v / 20)


def plugin_chain(p):
    """The stand-ins with the preset's settings (p: the preset as a dict from --show-preset)"""
    on = p["on"]
    st = []
    if on["denoise"]:
        st.append(lambda: LadspaPlugin(DF_LIB, "deep_filter_mono", FS, {"Attenuation Limit": p["denoise"]["limit"]}, BLOCK))
    e = p["eq"]
    st.append(lambda: LV2Plugin("http://calf.sourceforge.net/plugins/Equalizer8Band", FS, {
        "hp_active": 1, "hp_freq": e["hp"], "hp_mode": 0,
        "ls_active": 1, "ls_level": db(e["low"]), "ls_freq": 200,
        "p1_active": 1, "p1_level": db(e["pres"]), "p1_freq": 3000, "p1_q": 1.0,
        "hs_active": 1, "hs_level": db(e["air"]), "hs_freq": 10000}, BLOCK))
    st.append(lambda: LV2Plugin("http://calf.sourceforge.net/plugins/Deesser", FS, {"mode": 1}, BLOCK))
    x = p["exp"]
    st.append(lambda: LV2Plugin("http://calf.sourceforge.net/plugins/Gate", FS, {
        "threshold": db(x["thr"]), "ratio": x["ratio"], "range": db(-x["range"]), "attack": 3, "release": x["rel"]}, BLOCK))
    c = p["comp"]
    st.append(lambda: LV2Plugin("http://calf.sourceforge.net/plugins/MonoCompressor", FS, {
        "threshold": db(c["thr"]), "ratio": c["ratio"], "attack": c["att"], "release": c["rel"], "makeup": db(c["makeup"]),
        "knee": db(c["knee"])}, BLOCK))
    st.append(lambda: LV2Plugin("http://calf.sourceforge.net/plugins/Limiter", FS, {
        "limit": db(p["lim"]["ceil"]), "attack": 2, "release": p["lim"]["rel"], "auto_level": 0}, BLOCK))
    return [make() for make in st]


def run_plugins(p, x):
    plugs = plugin_chain(p)
    nb = len(x) // BLOCK
    y = np.zeros(nb * BLOCK, np.float32)
    period = BLOCK / FS
    start = time.perf_counter()
    for i in range(nb):
        buf = x[i * BLOCK:(i + 1) * BLOCK, None].astype(np.float32)
        for pl in plugs:
            out = pl.run(buf, [0.0])
            buf = out[:, :1] if out.shape[1] == 1 else out.mean(1, keepdims=True)
        y[i * BLOCK:(i + 1) * BLOCK] = buf[:, 0]
        wait = start + (i + 1) * period - time.perf_counter()
        if wait > 0:
            time.sleep(wait)
    for pl in plugs:
        pl.close()
    return y


def show_preset(cli, name, sets):
    import tomllib
    p = tomllib.loads(subprocess.run([cli, "x", "y", "--show-preset", name], capture_output=True, text=True, check=True).stdout)
    # the knobs given, as the chain applies them
    for k, v in sets.items():
        if k == "space" and v == 0:
            p["on"]["verb"] = False
    return p


def active_rms(x):
    fr = x[: len(x) // 480 * 480].reshape(-1, 480)
    e = (fr ** 2).mean(1)
    lv = 10 * np.log10(e + 1e-20)
    return np.sqrt(e[lv > lv.max() - 30].mean())


def align(x, y):
    lag = int(round(lag_ms(x, y, FS) * FS / 1e3))
    y = y[lag:]
    return np.concatenate([y, np.zeros(len(x) - len(y), np.float32)])[: len(x)]


def main():
    cli, out, data, tones = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3]), Path(sys.argv[4])
    out.mkdir(parents=True, exist_ok=True)
    work = out / "_work"
    work.mkdir(exist_ok=True)
    clips = []
    for f in ["f1_ipad_office1", "m3_ipad_livingroom1"]:
        x, _ = sf.read(data / "device" / f"{f}.wav", dtype="float64")
        clips.append((f"talk_{f.split('_')[0]}", "talk-omni", {}, (x if x.ndim == 1 else x[:, 0])[: 20 * FS],
                      f"說話：{'女聲' if f.startswith('f') else '男聲'}，iPad 錄音（全指向，真實房間），說話（全指向）"))
    for f in ["f6", "m3"]:
        s, _ = sf.read(data / "sing" / f"{f}.wav", dtype="float64")
        s = s[: 20 * FS]
        tone = np.resize(sf.read(tones / "tone_ipad_office1.wav", dtype="float64")[0], len(s))
        tone *= active_rms(s) / np.sqrt((tone ** 2).mean()) * db(-30)
        clips.append((f"sing_{f}", "sing", {"space": 0}, s + tone,
                      f"唱歌：{'女聲' if f.startswith('f') else '男聲'}（VocalSet，辦公室底噪 30 dB），唱歌（殘響關）"))
    rnd = random.Random(11)
    body, key = [], []
    for name, preset, sets, x, title in clips:
        x = (x * (db(-26) / active_rms(x))).astype(np.float32)   # a typical mic level: speech at -26 dBFS RMS
        x = np.clip(x, -1, 1)
        src = work / f"{name}.wav"
        sf.write(src, x, FS, subtype="FLOAT")
        cmd = [cli, str(src), str(work / f"{name}_ours.wav"), "--preset", preset]
        if sets:
            cmd += ["--set", ",".join(f"{k}={v}" for k, v in sets.items())]
        subprocess.run(cmd, check=True, capture_output=True)
        ours = align(x, sf.read(work / f"{name}_ours.wav", dtype="float32")[0])
        plug = align(x, run_plugins(show_preset(cli, preset, sets), x))
        vers = {"ours": ours, "plugins": plug}
        target = active_rms(ours)
        vers = {k: v * (target / active_rms(v)) for k, v in vers.items()}
        peak = max(np.abs(v).max() for v in vers.values())
        g = min(1.0, 0.95 / peak)
        order = ["ours", "plugins"]
        rnd.shuffle(order)
        body.append(f"<h2>{html.escape(title)}</h2>")
        sf.write(out / f"{name}__orig.wav", (x / max(np.abs(x).max(), 1e-9) * 0.5).astype(np.float32), FS, subtype="PCM_16")
        body.append(f'<div class=r><span>原始</span><audio controls preload=none src="{name}__orig.wav"></audio></div>')
        for lab, k in zip("AB", order):
            sf.write(out / f"{name}__{lab}.wav", (vers[k] * g).astype(np.float32), FS, subtype="PCM_16")
            body.append(f'<div class=r><span>{lab}</span><audio controls preload=none src="{name}__{lab}.wav"></audio></div>')
        key.append(f"{title}：" + "、".join(f"{lab} = {'我們的鏈' if k == 'ours' else '外掛版的鏈'}" for lab, k in zip("AB", order)))
        print(name, "done", flush=True)
    page = f"""<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>整條鏈試聽</title>
<style>body{{font-family:sans-serif;max-width:760px;margin:16px auto;padding:0 16px;background:#14161c;color:#e6e6ea}}h2{{font-size:1.05em;margin-top:28px}}.r{{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin:6px 0}}.r span{{min-width:80px}}audio{{width:100%;max-width:420px}}details{{margin-top:32px}}</style>
<h1>整條鏈試聽（盲聽）</h1>
<p>同一段聲音，一個過我們自己寫的整條鏈，一個過外掛版的鏈（DeepFilterNet 外掛、Calf 的 EQ、de-esser、gate、壓縮、限幅），兩邊盡量用同樣的設定，音量對齊。每段 A、B 的順序不一樣，答案在頁尾。建議戴耳機。</p>
<p>聽的重點：哪個比較自然、字有沒有被吃掉、句子之間的底噪、齒音、大聲時有沒有被壓扁。唱歌這組殘響都關掉（殘響另外一頁）。</p>
<p>原始的音量調小了，只是參考。素材：DAPS（非商業，只放區網）、VocalSet（CC BY 4.0）。</p>
{chr(10).join(body)}
<details><summary>答案</summary><p>{'<br>'.join(html.escape(k) for k in key)}</p></details>
"""
    (out / "index.html").write_text(page)


if __name__ == "__main__":
    main()
