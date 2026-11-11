# /// script
# dependencies = ["numpy", "soundfile", "scipy"]
# ///
"""Functional tests of the running engine on PipeWire, without a person: settings and their file, every route
(virtual mic, headphones, computer sound, their volumes, bypass, monitor modes, sync), each stage's switch, a restart
keeping the settings, and the mic or headphones unplugged and plugged back.

Needs: a stream-mic binary, PipeWire with pw-play, pw-cli and ffmpeg (PulseAudio input). Makes its own test devices
(null sinks "smt-mic" as the mic, "smt-spk" as the headphones) and its own settings folder, and removes them after.
Nothing else may be running a stream-mic. Usage: uv run tools/func_test.py BINARY [only-these-test-names...]

Windows: SMT_WIN=<ssh host> uv run tools/func_test.py stream-mic-test/stream-mic.exe (the binary's path on that machine,
under the user's folder): the engine in its test mode over ssh, devices as in smt_win.py. Devices cannot be removed
there, so the unplugging test only chooses the headphones in the page; the headphones get no computer sound from us
on Windows (Windows plays it there itself), so those checks are left out; one more test hears the real process loopback.
"""
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request

import numpy as np
import soundfile as sf

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
if os.environ.get("SMT_WIN"):
    import smt_win as smt_common  # noqa: E402
    sys.modules["smt_common"] = smt_common  # the import below takes the Windows names
else:
    import smt_common  # noqa: E402
MIC_HZ, PC_HZ, R, W, WIN = smt_common.MIC_HZ, smt_common.PC_HZ, smt_common.R, smt_common.W, smt_common.WIN
from smt_common import (Engine, Player, api, bursts, capture, capture_async, check, level, make_sink, post,  # noqa: E402
                        pw_ids, remove_sink, results, settings_break, settings_read, settings_restore, test, TESTS, tone,
                        vm_ids)

smt_common.init(sys.argv[1])
ONLY = sys.argv[2:]


# ---- the tests

def devices():
    """the engine's device list, as the page gets it (the first "devices" event)"""
    with urllib.request.urlopen(f"http://127.0.0.1:{smt_common.PORT}/events", timeout=5) as r:
        ev = None
        for line in r:
            line = line.decode().strip()
            if line.startswith("event:"):
                ev = line[6:].strip()
            elif line.startswith("data:") and ev == "devices":
                return json.loads(line[5:])


def raw_routes():
    """bypass on (the voice untouched), the usual routes"""
    post({"bypass": True, "loopback": True, "loopback_db": 0, "monitor": "processed", "monitor_db": 0, "pc_db": 0,
          "sync_ms": 0})


@test
def settings_and_file():
    code, s = api("GET", "/api/state")
    check("GET /api/state", code == 200 and "preset" in s and "builtins" in s)
    names = [b["name"] for b in s["builtins"]]
    check("three built-in presets", {"talk-omni", "talk-directional", "sing"} <= set(names), names)
    v = post({"monitor_db": -7.5, "loopback_db": -3, "pc_db": -2, "sync_ms": 12, "monitor": "direct"})
    check("plain values set", (v["monitor_db"], v["loopback_db"], v["pc_db"], v["sync_ms"], v["monitor"]) == (-7.5, -3, -2, 12, "direct"))
    v = post({"preset": {"comp": {"ratio": 5.5}}})
    check("one value deep in the preset", v["preset"]["comp"]["ratio"] == 5.5 and v["preset"]["comp"]["thr"] == s["preset"]["comp"]["thr"])
    name = v["preset"]["name"]
    check("the preset counts as edited", any(b["name"] == name and b["edited"] for b in v["builtins"]))
    v = post({"knobs": {"gate": -47}})
    check("gate knob is the expander threshold", v["preset"]["knobs"]["gate"] == -47 and v["preset"]["exp"]["thr"] == -47, (v["preset"]["knobs"]["gate"], v["preset"]["exp"]["thr"]))
    v = post({"preset": {"exp": {"thr": -52}}})
    check("expander threshold moves the gate knob", v["preset"]["knobs"]["gate"] == -52)
    other = "sing" if name != "sing" else "talk-omni"
    v = post({"preset_name": other})
    check("switch preset", v["preset"]["name"] == other and v["preset"]["comp"]["ratio"] != 5.5)
    v = post({"preset_name": name})
    check("switching back finds the changes", v["preset"]["comp"]["ratio"] == 5.5)
    v = post({"reset_preset": True})
    builtin_ratio = v["preset"]["comp"]["ratio"]
    check("reset drops them", builtin_ratio != 5.5 and not any(b["name"] == name and b["edited"] for b in v["builtins"]))
    v = post({"preset": {"name": "evil"}})
    check("the preset's name is not the page's to change", v["preset"]["name"] == name)
    code, _ = api("POST", "/api/state", {"preset_name": "nope"})
    check("unknown preset refused", code == 400)
    code, _ = api("POST", "/api/state", {"monitor": "loud"})
    check("bad value refused", code == 400)
    req = urllib.request.Request(f"http://127.0.0.1:{smt_common.PORT}/api/state", method="POST", data=b"{not json")
    try:
        urllib.request.urlopen(req, timeout=5)
        check("broken JSON refused", False)
    except urllib.error.HTTPError as e:
        check("broken JSON refused", e.code == 400)
    v = post({"bypass": True})
    f = settings_read(CFG)
    check("settings.json has the values", f["monitor_db"] == -7.5 and f["sync_ms"] == 12 and f["monitor"] == "direct")
    check("bypass never saved", "bypass" not in f)
    code, st = api("GET", "/api/stats")
    check("GET /api/stats", code == 200 and "send_under" in st)
    code, _ = api("GET", "/nope")
    check("unknown path 404", code == 404)
    with urllib.request.urlopen(f"http://127.0.0.1:{smt_common.PORT}/", timeout=5) as r:
        check("the page", r.status == 200 and b"stream-mic" in r.read())
    post({"bypass": False})


@test
def comp_styles():
    v = post({"preset_name": "sing"})
    thr, mk = v["preset"]["comp"]["thr"], v["preset"]["comp"]["makeup"]
    v = post({"preset": {"comp": {"style": "opto"}}})
    c = v["preset"]["comp"]
    check("optical style: its values", (c["style"], c["ratio"], c["knee"], c["att"], c["rel"], c["rel2"], c["slow"], c["memory"], c["auto_att"])
          == ("opto", 3.0, 12.0, 80.0, 40.0, 700.0, 35.0, 50.0, True), c)
    check("a style keeps the threshold and makeup", (c["thr"], c["makeup"]) == (thr, mk))
    v = post({"preset": {"comp": {"thr": -30}}})
    check("the threshold is not the style's", v["preset"]["comp"]["style"] == "opto")
    v = post({"preset": {"comp": {"rel2": 900}}})
    check("a value changed by hand: custom", v["preset"]["comp"]["style"] == "" and v["preset"]["comp"]["rel2"] == 900)
    v = post({"preset": {"comp": {"style": "clean"}}})
    c = v["preset"]["comp"]
    check("clean style: one release stage", (c["style"], c["rel2"], c["auto_att"], c["att"], c["rel"]) == ("clean", 0.0, False, 10.0, 120.0), c)
    code, _ = api("POST", "/api/state", {"preset": {"comp": {"style": "nope"}}})
    _, v = api("GET", "/api/state")
    check("unknown style refused, the style kept", code == 400 and v["preset"]["comp"]["style"] == "clean", code)
    post({"reset_preset": True})


@test
def restart_keeps_settings():
    global ENG
    post({"monitor_db": -4, "preset_name": "talk-omni"})
    post({"preset": {"comp": {"ratio": 3.3}}, "bypass": True})
    ENG.stop()
    ENG = Engine(CFG)
    _, v = api("GET", "/api/state")
    check("values back after a restart", v["monitor_db"] == -4 and v["preset"]["name"] == "talk-omni" and v["preset"]["comp"]["ratio"] == 3.3)
    check("bypass off after a restart", v["bypass"] is False)
    post({"reset_preset": True})
    ENG.stop()
    settings_break(CFG)
    ENG = Engine(CFG)
    code, v = api("GET", "/api/state")
    check("a broken settings file: starts on the defaults", code == 200 and v["preset"]["name"] == "talk-directional")
    ENG.stop()
    settings_restore(CFG)
    ENG = Engine(CFG)


@test
def routes():
    raw_routes()
    a = capture("stream-mic", 1.5)
    b = capture("smt-spk.monitor", 1.5)
    base_s_mic, base_s_pc = level(a, MIC_HZ), level(a, PC_HZ)
    base_h_mic, base_h_pc = level(b, MIC_HZ), level(b, PC_HZ)
    check("virtual mic: the mic at its level (bypass)", abs(base_s_mic - MIC_DB) < 0.5, f"{base_s_mic:.2f} dB, fed {MIC_DB:.2f}")
    check("virtual mic: the computer sound at its level", abs(base_s_pc - PC_DB) < 0.5, f"{base_s_pc:.2f} dB, fed {PC_DB:.2f}")
    check("headphones: the mic (bypass)", abs(base_h_mic - MIC_DB) < 0.5, f"{base_h_mic:.2f}")
    if WIN:
        # a trace at about -80 dB (55 under what is fed) has been seen: nothing anyone hears
        check("headphones: no computer sound from us (Windows plays it there)", base_h_pc < -60, f"{base_h_pc:.1f}")
    else:
        check("headphones: the computer sound", abs(base_h_pc - PC_DB) < 0.5, f"{base_h_pc:.2f}")
    check("both channels the same in the virtual mic", np.max(np.abs(a[:, 0] - a[:, 1])) < 1e-6)

    post({"loopback": False})
    a = capture("stream-mic", 1.5)
    check("loopback off: no computer sound in the virtual mic", level(a, PC_HZ) < -90, f"{level(a, PC_HZ):.1f}")
    check("loopback off: the voice stays", abs(level(a, MIC_HZ) - base_s_mic) < 0.3)
    if not WIN:
        b = capture("smt-spk.monitor", 1.5)
        check("loopback off: still in the headphones", abs(level(b, PC_HZ) - base_h_pc) < 0.3)
    post({"loopback": True, "loopback_db": -12})
    a = capture("stream-mic", 1.5)
    check("loopback -12 dB", abs(level(a, PC_HZ) - base_s_pc + 12) < 0.3, f"{level(a, PC_HZ) - base_s_pc:.2f}")
    post({"loopback_db": 0, "pc_db": -6})
    if not WIN:
        b = capture("smt-spk.monitor", 1.5)
        check("computer sound in the headphones -6 dB", abs(level(b, PC_HZ) - base_h_pc + 6) < 0.3, f"{level(b, PC_HZ) - base_h_pc:.2f}")
    a = capture("stream-mic", 1.5)
    check("... and not in the virtual mic", abs(level(a, PC_HZ) - base_s_pc) < 0.3)
    post({"pc_db": 0, "monitor_db": -10})
    b = capture("smt-spk.monitor", 1.5)
    check("monitor -10 dB", abs(level(b, MIC_HZ) - base_h_mic + 10) < 0.3, f"{level(b, MIC_HZ) - base_h_mic:.2f}")
    for mode in ("direct", "off"):
        post({"monitor": mode, "monitor_db": 0})
        b = capture("smt-spk.monitor", 1.5)
        check(f"monitor {mode}: no voice in the headphones", level(b, MIC_HZ) < -90, f"{level(b, MIC_HZ):.1f}")
        if not WIN:
            check(f"monitor {mode}: the computer sound stays", abs(level(b, PC_HZ) - base_h_pc) < 0.3)
        a = capture("stream-mic", 1.5)
        check(f"monitor {mode}: the virtual mic unchanged", abs(level(a, MIC_HZ) - base_s_mic) < 0.3)
    post({"monitor": "processed"})


@test
def bypass_is_the_raw_voice():
    """the voice untouched with bypass; changed by the chain without (the tone is quiet: the gate, EQ, compressor)"""
    raw_routes()
    post({"loopback": False})
    a = capture("stream-mic", 2.0)
    check("bypass: the tone at the level it was fed", abs(level(a, MIC_HZ) - MIC_DB) < 0.1, f"{level(a, MIC_HZ):.2f}")
    b = capture("smt-spk.monitor", 2.0)
    check("bypass: the headphones get the raw voice too", abs(level(b, MIC_HZ) - MIC_DB) < 0.1, f"{level(b, MIC_HZ):.2f}")
    post({"bypass": False})
    time.sleep(0.5)
    b = capture("stream-mic", 2.0)
    check("processing on: the level changes", abs(level(b, MIC_HZ) - level(a, MIC_HZ)) > 1.0, f"{level(a, MIC_HZ):.2f} -> {level(b, MIC_HZ):.2f}")
    post({"bypass": True})
    c = capture("stream-mic", 2.0)
    check("bypass again: back to the raw level", abs(level(c, MIC_HZ) - level(a, MIC_HZ)) < 0.1)
    post({"loopback": True})


def cycle_signal(path, secs=240):
    """0.5 s that repeats exactly, so any few seconds of it look the same: a loud vowel-like part (compressor,
    limiter), a hiss (de-esser), a middle part, and near silence (the gate)"""
    rng = np.random.default_rng(7)
    n = R // 2

    def band(lo, hi, k):
        X = np.fft.rfft(rng.standard_normal(k))
        f = np.fft.rfftfreq(k, 1 / R)
        X[(f < lo) | (f > hi)] = 0
        y = np.fft.irfft(X, k)
        return y / np.sqrt(np.mean(y ** 2))

    x = np.zeros(n)
    parts = [(0.00, 0.15, 150, 3000, 0.30), (0.15, 0.20, 5000, 9000, 0.12), (0.20, 0.35, 200, 4000, 0.06), (0.35, 0.50, 100, 8000, 0.0002)]
    for t0, t1, lo, hi, rms in parts:
        i0, i1 = int(t0 * R), int(t1 * R)
        ramp = np.minimum(1, np.minimum(np.arange(i1 - i0), np.arange(i1 - i0)[::-1]) / 96)
        x[i0:i1] = rms * band(lo, hi, i1 - i0) * ramp
    y = np.tile(x, int(secs * 2)).astype(np.float32)
    # Windows: --play loops a short file, 16-bit or float
    sf.write(path, np.stack([y, y], 1), R, subtype="FLOAT" if WIN else "PCM_24")


@test
def each_stage_switch():
    """each stage switched changes the virtual mic, beyond what two captures of the unchanged sound differ by.
    Not on Windows: the waveform there moves with the two clocks (the test mic on the system's, the outputs on the
    interface's, followed by the resampler), and level patterns differ by 0.3-2 dB between captures, as much as some
    switches change. The chain is the same code on both (stream-mic-cli gives the same samples on both) and the
    Windows audio paths are what the other tests here check. The interface's own loop (SMT_WIN_RIG=revelator) did
    not help at 44.1 kHz: its Stream Mix A dropped 32 samples at a time (at 48 kHz it is sample exact), and our
    resampler starts some 900 ppm off and takes about 30 s to settle (2026-11-08)"""
    if WIN:
        print("  skipped on Windows")
        return
    post({"bypass": False, "loopback": False, "reset_preset": True, "preset_name": "talk-directional"})
    MIC.stop()
    sig = os.path.join(W, "cycle.wav")
    cycle_signal(sig, 10 if WIN else 240)
    sp = Player("smt-mic", sig)

    P = R // 2

    def cycle(x):
        """the capture folded onto one period (the output repeats while nothing changes)"""
        m = x.mean(1)
        k = len(m) // P
        return m[:k * P].reshape(k, P).mean(0)

    def diff_db(c1, c2):
        """how far c2 is from c1 at the best circular alignment, dB under c1 (-inf: the same)"""
        xc = np.fft.irfft(np.fft.rfft(c1) * np.conj(np.fft.rfft(c2)), P)
        k = int(np.argmax(xc))
        r = c1 - np.roll(c2, k)
        return 10 * np.log10(np.sum(r ** 2) / np.sum(c1 ** 2) + 1e-20)

    try:
        time.sleep(1.5)
        base = cycle(capture("stream-mic", 4.3))
        same = diff_db(base, cycle(capture("stream-mic", 4.3)))
        check("two captures of the same sound agree", same < -40, f"{same:.1f} dB")
        _, s0 = api("GET", "/api/state")
        for st in ("eq", "deess", "exp", "comp", "lim", "verb", "denoise"):
            was = s0["preset"]["on"][st]
            if st == "verb":
                # the presets for talking mix none in (the 空間 knob at 0): some, to hear it switch
                post({"preset": {"verb": {"mix": 0.3}}})
                time.sleep(1.5)
                base = cycle(capture("stream-mic", 4.3))
            post({"preset": {"on": {st: not was}}})
            time.sleep(1.5 if st != "denoise" else 2.5)
            d = diff_db(base, cycle(capture("stream-mic", 4.3)))
            post({"preset": {"on": {st: was}}})
            time.sleep(1.5 if st != "denoise" else 2.5)
            d0 = api("GET", "/api/stats")[1]
            back = diff_db(base, cycle(capture("stream-mic", 4.3)))
            if back > max(same + 6, -40):
                # pw-play can skip under load (pw-record did, obs_rec_test.sh), and the input is then not periodic
                # for that capture: once more
                back = diff_db(base, cycle(capture("stream-mic", 4.3)))
            d1 = api("GET", "/api/stats")[1]
            drops = sum(d1[k] - d0[k] for k in ("send_drop", "send_under", "dn_under"))
            what = "off" if was else "on"
            check(f"{st} {what}: the sound changes", d > same + 15, f"{d:.1f} dB from before, unchanged {same:.1f}")
            check(f"{st} back: as before", back < max(same + 6, -40), f"{back:.1f} dB, drops and gaps meanwhile {drops}")
        post({"reset_preset": True})
    finally:
        sp.stop()
        MIC.__init__("smt-mic", MIC_TONE)
        post({"loopback": True})


@test
def sync_delays_the_computer_sound():
    raw_routes()
    PC.stop()
    MIC.stop()
    pb = Player("stream-mic-pc-audio", PC_BURSTS)
    try:
        time.sleep(0.8)
        # one capture across the change: the bursts' phase in the virtual mic moves by sync_ms. Over ssh the capture
        # starts up to a second or so late: longer, the change later, the last 2.2 s compared with the first 2.3
        n = 8 if WIN else 6
        p = capture_async("stream-mic", n)
        time.sleep(4.0 if WIN else 3.0)
        api("POST", "/api/state", {"sync_ms": 80})
        x = p.wait()
        m = np.abs(x.mean(1))

        def onsets(seg, t0):
            out, i = [], 0
            while i < len(seg):
                if seg[i] > 0.05:
                    out.append((t0 + i) / R)
                    i += int(0.3 * R)
                else:
                    i += 1
            return out

        a = onsets(m[int(0.5 * R):int(2.8 * R)], int(0.5 * R))
        b = onsets(m[int((n - 2.4) * R):int((n - 0.2) * R)], int((n - 2.4) * R))
        # the bursts' period as captured (Windows: up to 0.3 % off 0.5 s, the clocks), from their spacing
        per = float(np.median(np.concatenate([np.diff(a), np.diff(b)]))) if WIN else 0.5
        pa = np.median([(t % per) for t in a])
        pb_ = np.median([(t % per) for t in b])
        shift = ((pb_ - pa) % per) * 1000
        check("sync 80 ms delays the computer sound by 80 ms", abs(shift - 80) < 3, f"{shift:.1f} ms")
    finally:
        pb.stop()
        post({"sync_ms": 0})
        PC.__init__("stream-mic-pc-audio", PC_TONE)
        MIC.__init__("smt-mic", MIC_TONE)


@test
def unplug_and_plug_back():
    raw_routes()
    if WIN:
        # no devices to remove: a headphones choice that is not there, then the real one again
        post({"out": "no-such-output"})
        time.sleep(1.5)
        d = devices()
        check("a missing headphones choice is reported", d.get("out_missing") == "no-such-output", d.get("out_missing"))
        a = capture("stream-mic", 1.5)
        check("... and the virtual mic goes on", abs(level(a, MIC_HZ) - MIC_DB) < 0.5, f"{level(a, MIC_HZ):.1f}")
        out = next(o["name"] for o in d["outs"] if smt_common.DEV["smt-spk"] in o["desc"])
        post({"out": out})
        time.sleep(4.0)  # the new output's clock followed from the start again
        b = capture("smt-spk.monitor", 1.5)
        check("headphones chosen in the page: sound there", abs(level(b, MIC_HZ) - MIC_DB) < 0.5, f"{level(b, MIC_HZ):.1f}")
        check("engine still running", ENG.running())
        return
    # OBS never finds a virtual mic again once it has gone: it must stay the same node throughout
    ids = vm_ids()
    # the headphones
    remove_sink("smt-spk")
    time.sleep(2.0)
    _, s = api("GET", "/api/state")
    a = capture("stream-mic", 1.5)
    check("headphones gone: the virtual mic goes on", abs(level(a, MIC_HZ) - MIC_DB) < 0.5, f"{level(a, MIC_HZ):.1f}")
    make_sink("smt-spk")
    time.sleep(3.0)
    b = capture("smt-spk.monitor", 1.5)
    check("headphones back: they get the sound again", abs(level(b, MIC_HZ) - MIC_DB) < 0.5, f"{level(b, MIC_HZ):.1f}")
    # the mic
    MIC.stop()
    remove_sink("smt-mic")
    time.sleep(2.0)
    a = capture("stream-mic", 1.5)
    check("mic gone (no other): the virtual mic stays", pw_ids("stream-mic") != [] and ENG.running(), f"{level(a, MIC_HZ):.1f}")
    check("mic gone: the computer sound still in the virtual mic", abs(level(a, PC_HZ) - PC_DB) < 0.5, f"{level(a, PC_HZ):.1f}")
    b = capture("smt-spk.monitor", 1.5)
    check("mic gone: ... and in the headphones", abs(level(b, PC_HZ) - PC_DB) < 0.5, f"{level(b, PC_HZ):.1f}")
    make_sink("smt-mic")
    MIC.__init__("smt-mic", MIC_TONE)
    time.sleep(3.0)
    a = capture("stream-mic", 1.5)
    check("mic back: its sound in the virtual mic again", abs(level(a, MIC_HZ) - MIC_DB) < 0.5, f"{level(a, MIC_HZ):.1f}")
    check("engine still running", ENG.running())
    check("the virtual mic and 電腦聲音 never made again", vm_ids() == ids, f"{ids}")
    post({"out": "smt-mic"})
    time.sleep(1.0)
    post({"out": "smt-spk"})
    time.sleep(1.5)
    b = capture("smt-spk.monitor", 1.5)
    check("headphones chosen in the page: sound there", abs(level(b, MIC_HZ) - MIC_DB) < 0.5, f"{level(b, MIC_HZ):.1f}")
    check("... and the virtual mic still the same node", vm_ids() == ids)


@test
def pipewire_restarts():
    """PipeWire restarted under a running engine (a WirePlumber rule changed, an update): it connects again by
    itself, with the virtual mic, the mic and the headphones, and the sound goes through as before (2026-11-09: it
    stayed on the dead connection with no devices until stream-mic itself was restarted). Restarts this user's
    PipeWire: other sound on the machine stops for a moment. SMT_NO_PW_RESTART=1 leaves it out."""
    if WIN or os.environ.get("SMT_NO_PW_RESTART"):
        return
    raw_routes()
    MIC.stop()
    PC.stop()
    subprocess.run(["systemctl", "--user", "restart", "pipewire", "pipewire-pulse", "wireplumber"], check=True)
    for _ in range(100):  # PipeWire answering again
        if subprocess.run(["pw-cli", "info", "0"], capture_output=True).returncode == 0:
            break
        time.sleep(0.1)
    make_sink("smt-mic")
    make_sink("smt-spk")
    MIC.__init__("smt-mic", MIC_TONE)
    PC.__init__("stream-mic-pc-audio", PC_TONE)
    for _ in range(100):
        if pw_ids("stream-mic") and pw_ids("stream-mic-pc-audio"):
            break
        time.sleep(0.1)
    # the computer sound's player started before our sink came back would have gone elsewhere: once more
    PC.stop()
    PC.__init__("stream-mic-pc-audio", PC_TONE)
    time.sleep(3.0)
    check("PipeWire restarted: the engine still running", ENG.running())
    check("... the virtual mic and 電腦聲音 there again", all(vm_ids()), f"{vm_ids()}")
    a = capture("stream-mic", 1.5)
    # (with no "stream-mic" node the capture takes the default source: the levels count only with the node there)
    vm = all(vm_ids())
    check("... the mic in the virtual mic at its level", vm and abs(level(a, MIC_HZ) - MIC_DB) < 0.5, f"{level(a, MIC_HZ):.1f}")
    check("... the computer sound in the virtual mic at its level", vm and abs(level(a, PC_HZ) - PC_DB) < 0.5, f"{level(a, PC_HZ):.1f}")
    b = capture("smt-spk.monitor", 1.5)
    check("... the headphones get the mic", abs(level(b, MIC_HZ) - MIC_DB) < 0.5, f"{level(b, MIC_HZ):.1f}")


@test
def settings_saved_from_the_desktop():
    """Windows: started as from the Start menu (no HOME), the settings go to %APPDATA%\\stream-mic and come back"""
    global ENG
    if not WIN:
        return
    ENG.stop()
    D = os.path.join(W, "appdata")
    ENG = Engine(D, desktop=True)
    try:
        post({"monitor_db": -9})
        f = smt_common.settings_read(D)
        check("desktop start: settings saved under APPDATA", f["monitor_db"] == -9)
        ENG.stop()
        ENG = Engine(D, desktop=True)
        _, v = api("GET", "/api/state")
        check("desktop start: and read back", v["monitor_db"] == -9)
    finally:
        ENG.stop()
        ENG = Engine(CFG)


@test
def process_loopback_hears_programs():
    """Windows: without the test's computer-sound device, the real process loopback: another program's sound (the
    computer-sound player, wherever it plays) reaches the virtual mic"""
    global ENG
    if not WIN:
        return
    raw_routes()
    ENG.stop()
    # the mic's player is another program too: heard both ways the loop detector (rightly) mutes the computer sound
    MIC.stop()
    ENG = Engine(CFG, test_pc=False)
    try:
        raw_routes()
        time.sleep(1.0)
        a = capture("stream-mic", 1.5)
        check("process loopback: another program's sound in the virtual mic", abs(level(a, PC_HZ) - PC_DB) < 1.0, f"{level(a, PC_HZ):.2f} dB, fed {PC_DB:.2f}")
        check("process loopback: reported working", devices().get("loopback_ok") is True)
    finally:
        ENG.stop()
        MIC.__init__("smt-mic", MIC_TONE)
        ENG = Engine(CFG)


@test
def counters_clean():
    _, st = api("GET", "/api/stats")
    check("no underruns", st["send_under"] == 0 and st["mon_under"] == 0, {k: st[k] for k in ("send_under", "mon_under", "send_drop", "mon_drop", "dn_under")})


# ---- run

if not WIN and pw_ids("stream-mic"):
    sys.exit("another stream-mic is running (its virtual mic is there): stop it first")
CFG = os.path.join(W, "cfg")
os.makedirs(CFG)
MIC_TONE, PC_TONE, PC_BURSTS = (os.path.join(W, n) for n in ("mic.wav", "pc.wav", "bursts.wav"))
tone(MIC_TONE, MIC_HZ, 0.1)
tone(PC_TONE, PC_HZ, 0.05)
bursts(PC_BURSTS, PC_HZ, 0.3)
MIC_DB, PC_DB = 20 * np.log10(0.1), 20 * np.log10(0.05)
make_sink("smt-mic")
make_sink("smt-spk")
ENG = Engine(CFG)
MIC = Player("smt-mic", MIC_TONE)
PC = Player("stream-mic-pc-audio", PC_TONE)
time.sleep(1.0)
try:
    # the counters before unplugging (that underruns, as it should), and before the engine restarted without test devices
    # PipeWire's restart last of all: every node is made anew there
    TESTS.sort(key=lambda f: (f in (process_loopback_hears_programs, settings_saved_from_the_desktop)) * 2 + (f is unplug_and_plug_back)
               + (f is pipewire_restarts) * 4)
    for fn in TESTS:
        if ONLY and fn.__name__ not in ONLY:
            continue
        print(fn.__name__, flush=True)
        try:
            fn()
        except Exception as e:  # a test that breaks is a failure, the rest still run
            check(fn.__name__ + " ran", False, repr(e))
finally:
    MIC.stop()
    PC.stop()
    ENG.stop()
    if not WIN:
        remove_sink("smt-mic")
        remove_sink("smt-spk")
    bad = [r for r in results if not r[1]]
    print(f"\n{len(results) - len(bad)} passed, {len(bad)} failed" + "".join(f"\n  FAIL {n} ({d})" for n, _, d in bad))
    # the sounds are big: gone either way; the engine's log stays when something failed
    for f in os.listdir(W):
        if f.endswith(".wav"):
            os.remove(os.path.join(W, f))
    if bad:
        print(f"engine log: {os.path.join(W, 'engine.log')}")
    else:
        shutil.rmtree(W, ignore_errors=True)
