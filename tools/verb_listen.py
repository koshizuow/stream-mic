"""Blind listening set for the reverb (build-plan.md step 3): our FDN, Dattorro's plate (refverb.py) and
Dragonfly Plate (nested), all with the same T60, damping and low cut, onsets lined up (our FDN's wet is
centred about 29 ms in, so the other two get that much pre-delay), and the wet level matched (wet RMS
12 dB under the dry). Writes WAVs and index.html; the labels are shuffled per clip, the key is at the end.

Usage: uv run python verb_listen.py CLI OUT_DIR SPEECH_DIR SING_DIR
"""
import html
import random
import subprocess
import sys
from pathlib import Path

import numpy as np
import soundfile as sf

from refverb import reverb as dattorro

FS = 48000
HERE = Path(__file__).parent
T60, LOWCUT, DAMP = 1.6, 250.0, 7000.0
WET_DB = -12.0
CLIPS = [("speech_f1", "SPEECH", "f1.wav", 0, 12), ("speech_m1", "SPEECH", "m1.wav", 0, 12),
         ("sing_f6", "SING", "f6.wav", 0, 16), ("sing_m3", "SING", "m3.wav", 0, 16)]
TITLES = {"speech_f1": "說話：女聲（DAPS f1，英文朗讀）", "speech_m1": "說話：男聲（DAPS m1）",
          "sing_f6": "歌聲：女聲（VocalSet f6，古典唱法）", "sing_m3": "歌聲：男聲（VocalSet m3）"}


def rms(v):
    return np.sqrt(np.mean(v ** 2))


def ours(cli, src, work):
    out = work / "ours.wav"
    subprocess.run([cli, str(src), str(out), "--verb", f"t60={T60},pre=0,lowcut={LOWCUT},damp={DAMP},mix=1,mixdry=1",
                    "--stereo"], check=True, capture_output=True)
    y = sf.read(out, dtype="float64")[0]
    x = sf.read(src, dtype="float64")[0]
    return y - x[:, None]


def plate(src, work):
    out = work / "dragonfly.wav"
    ctl = dict(dry_level=0, early_level=100, algorithm=1, predelay=29, decay=T60, low_cut=200, high_cut=DAMP,
               early_damp=DAMP, width=100)
    subprocess.run([sys.executable, str(HERE / "lv2host.py"), "urn:dragonfly:plate", str(src), str(out),
                    *[f"{k}={v}" for k, v in ctl.items()]], check=True, capture_output=True)
    return sf.read(out, dtype="float64")[0]


def main():
    cli, out = sys.argv[1], Path(sys.argv[2])
    dirs = {"SPEECH": Path(sys.argv[3]), "SING": Path(sys.argv[4])}
    out.mkdir(parents=True, exist_ok=True)
    work = out / "_work"
    work.mkdir(exist_ok=True)
    rnd = random.Random(7)
    body, key = [], []
    for name, d, f, start, secs in CLIPS:
        x, fs = sf.read(dirs[d] / f, dtype="float64")
        x = (x if x.ndim == 1 else x[:, 0])[start * FS:(start + secs) * FS]
        x = x / np.abs(x).max() * 10 ** (-6 / 20)
        src = work / f"{name}.wav"
        sf.write(src, x.astype(np.float32), FS, subtype="FLOAT")
        wets = {"ours": ours(cli, src, work)}
        yl, yr = dattorro(x, FS, T60, 20.0, LOWCUT, DAMP)  # Dattorro's wet starts about 9 ms in: +20 lines it up
        wets["dattorro"] = np.stack([yl, yr], 1)
        wets["dragonfly"] = plate(src, work)[: len(x)]
        versions = {"dry": np.stack([x, x], 1)}
        for k, w in wets.items():
            w = w[: len(x)]
            w = w * (rms(x) * 10 ** (WET_DB / 20) / rms(w))
            versions[k] = x[:, None] + w
        peak = max(np.abs(v).max() for v in versions.values())
        labels = ["A", "B", "C"]
        order = ["ours", "dattorro", "dragonfly"]
        rnd.shuffle(order)
        body.append(f"<h2>{html.escape(TITLES[name])}</h2>")
        for lab, k in [("乾聲（沒有殘響）", "dry")] + list(zip(labels, order)):
            fn = f"{name}__{lab if k != 'dry' else 'dry'}.wav"
            sf.write(out / fn, (versions[k] / peak * 0.9).astype(np.float32), FS, subtype="PCM_16")
            body.append(f'<div class=r><span>{html.escape(lab)}</span><audio controls preload=none src="{fn}"></audio></div>')
        names = {"ours": "我們的 FDN", "dattorro": "Dattorro 板式（參考實作）", "dragonfly": "Dragonfly Plate（nested）"}
        key.append(f"{TITLES[name]}：" + "、".join(f"{lab} = {names[k]}" for lab, k in zip(labels, order)))
    page = f"""<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1"><title>殘響試聽</title>
<style>body{{font-family:sans-serif;max-width:760px;margin:16px auto;padding:0 16px;background:#14161c;color:#e6e6ea}}h2{{font-size:1.05em;margin-top:28px}}.r{{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin:6px 0}}.r span{{min-width:160px}}audio{{width:100%;max-width:420px}}details{{margin-top:32px}}</style>
<h1>殘響試聽（盲聽）</h1>
<p>三種殘響，長度（T60）都是 {T60} 秒、高頻阻尼 {DAMP / 1000:.0f} kHz、殘響低切 {LOWCUT:.0f} Hz，殘響開始的時間對齊，殘響音量都比乾聲小 {-WET_DB:.0f} dB。每段的 A、B、C 順序不一樣。建議戴耳機。</p>
<p>聽的重點：殘響開頭有沒有顆粒感或一個個回聲、尾巴有沒有金屬聲或嗡嗡聲、字有沒有被糊掉、哪個最自然。殘響量故意放得比直播用的多，方便聽出差別。</p>
<p>素材：DAPS（非商業授權，只放區網）、VocalSet（CC BY 4.0）。</p>
{chr(10).join(body)}
<details><summary>答案</summary><p>{'<br>'.join(html.escape(k) for k in key)}</p></details>
"""
    (out / "index.html").write_text(page)
    print("\n".join(key))


if __name__ == "__main__":
    main()
