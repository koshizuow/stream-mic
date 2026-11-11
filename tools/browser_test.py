# /// script
# dependencies = ["numpy", "soundfile", "playwright==1.49.1"]
# ///
"""The page and the virtual mic in real browsers (Playwright's Chromium and Firefox), without a person:

- capture: the browser lists "stream-mic" as a microphone and records it (getUserMedia, WebAudio): what Discord in
  a browser, or Discord's Electron app (Chromium), would do
- page: every control on the settings page reaches the engine (/api/state after each), the meters move, a reload
  shows what was set, no errors in the console

Needs what func_test.py needs, plus `uv run --with playwright==1.49.1 python -m playwright install chromium firefox`.
Usage: uv run tools/browser_test.py BINARY [chromium|firefox ...]
"""
import os
import shutil
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import smt_common  # noqa: E402
from smt_common import MIC_HZ, PC_HZ, W, Engine, Player, api, check, make_sink, pw_ids, remove_sink, results, tone  # noqa: E402

from playwright.sync_api import sync_playwright  # noqa: E402

smt_common.init(sys.argv[1], port=8796)
BROWSERS = sys.argv[2:] or ["chromium", "firefox"]
URL = f"http://127.0.0.1:{smt_common.PORT}/"


def state():
    return api("GET", "/api/state")[1]


def wait_for(f, secs=3.0):
    t = time.time()
    while time.time() - t < secs:
        v = f()
        if v:
            return v
        time.sleep(0.1)
    return f()


# the page's JS: find stream-mic, record a while, say how loud and at what frequency
CAPTURE_JS = """async (secs) => {
  const pre = await navigator.mediaDevices.getUserMedia({ audio: true });
  pre.getTracks().forEach((t) => t.stop());
  const devs = await navigator.mediaDevices.enumerateDevices();
  const ins = devs.filter((d) => d.kind === 'audioinput');
  const dev = ins.find((d) => /stream-mic/.test(d.label) && !/電腦聲音|monitor/i.test(d.label));
  if (!dev) return { labels: ins.map((d) => d.label) };
  const st = await navigator.mediaDevices.getUserMedia({ audio: { deviceId: { exact: dev.deviceId },
    echoCancellation: false, noiseSuppression: false, autoGainControl: false } });
  const ctx = new AudioContext({ sampleRate: 48000 });
  const src = ctx.createMediaStreamSource(st);
  const an = ctx.createAnalyser(); an.fftSize = 32768; an.smoothingTimeConstant = 0;
  src.connect(an);
  await new Promise((r) => setTimeout(r, secs * 1000));
  const f = new Float32Array(an.frequencyBinCount); an.getFloatFrequencyData(f);
  const t = new Float32Array(an.fftSize); an.getFloatTimeDomainData(t);
  const rms = Math.sqrt(t.reduce((a, x) => a + x * x, 0) / t.length);
  const bin = (hz) => Math.max(...[-2, -1, 0, 1, 2].map((k) => f[Math.round(hz / (ctx.sampleRate / an.fftSize)) + k]));
  st.getTracks().forEach((x) => x.stop());
  await ctx.close();
  return { labels: ins.map((d) => d.label), label: dev.label, rms_db: 20 * Math.log10(rms + 1e-12),
           mic_db: bin(%MIC%), pc_db: bin(%PC%), off_db: bin(700), rate: ctx.sampleRate };
}""".replace("%MIC%", str(MIC_HZ)).replace("%PC%", str(PC_HZ))


def capture_test(name, page):
    r = page.evaluate(CAPTURE_JS, 2)
    check(f"{name}: lists stream-mic as a microphone", "label" in r, r.get("labels"))
    if "label" not in r:
        return
    check(f"{name}: records it (sound, not silence)", r["rms_db"] > -40, f"{r['rms_db']:.1f} dBFS")
    # both tones in it (the mic's and the computer's), nothing between them
    check(f"{name}: the voice and the computer sound in it", r["mic_db"] > r["off_db"] + 30 and r["pc_db"] > r["off_db"] + 30,
          f"440 Hz {r['mic_db']:.0f}, 1 kHz {r['pc_db']:.0f}, 700 Hz {r['off_db']:.0f} dB")


def set_range(page, sel, v):
    """move a slider the way a hand does: value, then input and change"""
    page.eval_on_selector(sel, "(e, v) => { e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); "
                               "e.dispatchEvent(new Event('change', { bubbles: true })); }", v)


def shown_hidden(page):
    """elements the page marked hidden that still take room on screen (a CSS display beating the attribute)"""
    return page.eval_on_selector_all("[hidden]", "es => es.filter((e) => e.getClientRects().length > 0).map((e) => e.id || e.className || e.tagName)")


def page_test(name, page):
    """the new page (ui/a.html, b.html, app.js): every control reaches the engine"""
    errors = []
    page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    page.on("pageerror", lambda e: errors.append(str(e)))
    api("POST", "/api/state", {"ui": {"layout": "b", "lang": "zh"}})
    page.goto(URL)
    page.wait_for_function("() => document.getElementById('state')?.textContent === '運作中'", timeout=8000)
    check(f"{name}: page connects to the engine (layout B, 繁中)", page.eval_on_selector("body", "b => b.dataset.layout") == "b")
    seen = shown_hidden(page)
    check(f"{name}: nothing marked hidden is on screen", not seen, seen)
    check(f"{name}: three modes", page.locator("#modes button").count() == 3)
    check(f"{name}: the app's box hidden in a browser", page.locator("#appBox").is_hidden())

    for p in ("talk-omni", "sing", "talk-directional"):
        page.click(f'#modes button[data-n="{p}"]')
        check(f"{name}: mode {p}", wait_for(lambda: state()["preset"]["name"] == p))

    page.click("#bypass")
    check(f"{name}: bypass on", wait_for(lambda: state()["bypass"] is True))
    check(f"{name}: ... the button says so", wait_for(lambda: "原音中" in page.text_content("#bypass")))
    page.click("#bypass")
    check(f"{name}: ... and off", wait_for(lambda: state()["bypass"] is False))
    page.click("body", position={"x": 5, "y": 300})
    page.keyboard.down("Space")
    held = wait_for(lambda: state()["bypass"] is True)
    page.keyboard.up("Space")
    check(f"{name}: space held is bypass, let go is back", held and wait_for(lambda: state()["bypass"] is False))

    for k, v in (("denoise", 40), ("body", -30), ("space", 25), ("sib", 70), ("gate", -50)):
        set_range(page, f'#sliders input[data-k="{k}"]', v)
        check(f"{name}: slider {k}", wait_for(lambda: state()["preset"]["knobs"][k] == v), state()["preset"]["knobs"][k])
    check(f"{name}: the mode shows 改過", wait_for(lambda: "改過" in page.text_content("#modes")))
    api("POST", "/api/state", {"reset_preset": True})

    # the computer's sound and the monitor
    lb = state()["loopback"]
    page.click("#lbOn")
    check(f"{name}: loopback switch", wait_for(lambda: state()["loopback"] is (not lb)))
    page.click("#lbOn")
    wait_for(lambda: state()["loopback"] is lb)
    for el, key, v in (("lbDb", "loopback_db", -6.5), ("pcDb", "pc_db", -3), ("monDb", "monitor_db", 2.5)):
        set_range(page, f"#{el}", v)
        check(f"{name}: {key}", wait_for(lambda: state()[key] == v))
    for m in ("direct", "off", "processed"):
        page.click(f'#mon button[data-v="{m}"]')
        check(f"{name}: monitor {m}", wait_for(lambda: state()["monitor"] == m))
        check(f"{name}: ... its volume only when processed", wait_for(lambda: page.locator("#monRow").is_hidden() == (m != "processed")))

    # the chain: each stage's switch, 耳機, every parameter
    for st in ("denoise", "eq", "deess", "exp", "comp", "verb", "lim"):
        page.click(f'#chain .mod[data-s="{st}"]')
        page.wait_for_selector("#detail [data-on]")
        was = state()["preset"]["on"][st]
        page.click("#detail [data-on]")
        ok = wait_for(lambda: state()["preset"]["on"][st] == (not was))
        page.click("#detail [data-on]")
        check(f"{name}: {st} switch", ok and wait_for(lambda: state()["preset"]["on"][st] == was))
        mon = state()["preset"]["monitor"][st]
        page.click("#detail [data-mon]")
        check(f"{name}: {st} 耳機", wait_for(lambda: state()["preset"]["monitor"][st] == (not mon)))
        params = page.eval_on_selector_all('#detail .lvl[data-p]:not([hidden]) input', "es => es.map((e) => [e.closest('.lvl').dataset.p, +e.min, +e.max, +e.step])")
        for pid, lo, hi, step in params:
            v = round(lo + round((hi - lo) * 0.37 / step) * step, 6)
            set_range(page, f'#detail .lvl[data-p="{pid}"] input', v)
            ok = wait_for(lambda: abs(state()["preset"][st][pid] - v) < 1e-6)
            check(f"{name}: {st}.{pid}", ok, f"set {v}, engine {state()['preset'][st][pid]}")
        if st == "comp":
            # its styles and the automatic attack
            page.click('#cstyles button[data-v="opto"]')
            check(f"{name}: comp style opto", wait_for(lambda: state()["preset"]["comp"]["style"] == "opto" and state()["preset"]["comp"]["rel2"] == 700))
            check(f"{name}: ... its button lit", wait_for(lambda: page.get_attribute('#cstyles button[data-v="opto"]', "aria-pressed") == "true"))
            page.click("#autoAtt")
            check(f"{name}: comp automatic attack off, custom", wait_for(lambda: state()["preset"]["comp"]["auto_att"] is False and state()["preset"]["comp"]["style"] == ""))
            page.click('#cstyles button[data-v="two-knob"]')
            check(f"{name}: comp two knobs", wait_for(lambda: state()["preset"]["comp"]["two_knob"] is True))
            check(f"{name}: ... Amount shown, Ratio hidden", wait_for(lambda: page.locator('#detail .lvl[data-p="amount"]').is_visible() and page.locator('#detail .lvl[data-p="ratio"]').is_hidden()))
            page.click("#limMode")
            check(f"{name}: ... Limit", wait_for(lambda: state()["preset"]["comp"]["limit"] is True))
            if os.environ.get("SHOT"):
                page.wait_for_timeout(300)
                page.locator("#detail").screenshot(path=os.environ["SHOT"] + f"-two-{name}.png")
                page.click('#cstyles button[data-v="opto"]')
                wait_for(lambda: state()["preset"]["comp"]["style"] == "opto")
                page.wait_for_timeout(300)
                page.locator("#detail").screenshot(path=os.environ["SHOT"] + f"-{name}.png")
        page.click(f'#chain .mod[data-s="{st}"]')
    api("POST", "/api/state", {"reset_preset": True, "monitor_db": 0, "pc_db": 0, "loopback_db": 0})

    # the devices: the page lists the test ones, and choosing one moves the engine
    page.wait_for_function("() => document.querySelectorAll('#devOut option').length > 0", timeout=5000)
    opts = page.eval_on_selector_all("#devOut option", "os => os.map((o) => o.value)")
    check(f"{name}: headphones list has the test sink", "smt-spk" in opts, opts)
    page.select_option("#devOut", "smt-mic")
    check(f"{name}: choose headphones", wait_for(lambda: state()["out"] == "smt-mic"))
    page.select_option("#devOut", "smt-spk")
    wait_for(lambda: state()["out"] == "smt-spk")

    # the levels reach the page: the voice's loudness readout and the scope
    check(f"{name}: the readout shows the voice", wait_for(lambda: "−∞" not in (page.text_content("#bal") or "−∞").split("LUFS")[1][:12], 5))
    check(f"{name}: the scope draws", page.eval_on_selector("#scope", "c => c.width > 0"))

    # auto setup from the page: a panel, started and cancelled
    page.click("#auto")
    check(f"{name}: 自動設定 starts", wait_for(lambda: api("GET", "/api/autoset")[1]["phase"] in ("quiet", "talk")))
    check(f"{name}: ... its panel shows", page.locator("#autoBox").is_visible())
    page.click("#auCancel")
    check(f"{name}: ... and cancels", wait_for(lambda: api("GET", "/api/autoset")[1]["phase"] == "cancelled", 5))
    page.click("#auClose")

    # A / B from the header: the setting, then the other page
    page.click('#layoutSw button[data-v="a"]')
    check(f"{name}: switch to A (kept in the settings)", wait_for(lambda: state()["ui"]["layout"] == "a"))
    page.wait_for_function("() => document.body?.dataset.layout === 'a' && document.getElementById('state')?.textContent === '運作中'", timeout=8000)
    check(f"{name}: page A loads", page.locator("#knobs .knob").count() == 5)
    sib = state()["preset"]["knobs"]["sib"]
    page.focus('#knobs .knob[data-k="sib"]')
    page.keyboard.press("ArrowUp")
    check(f"{name}: a knob by the keyboard", wait_for(lambda: state()["preset"]["knobs"]["sib"] == sib + 2))
    seen = shown_hidden(page)
    check(f"{name}: A: nothing marked hidden is on screen", not seen, seen)
    # the language from the settings
    page.click("#gear")
    page.click('.pop button[data-v="en"]')
    check(f"{name}: language kept in the settings", wait_for(lambda: state()["ui"]["lang"] == "en"))
    page.wait_for_function("() => document.getElementById('state')?.textContent === 'Running'", timeout=8000)
    check(f"{name}: the page in English", "Computer audio" in page.text_content("#side"))
    api("POST", "/api/state", {"ui": {"layout": "a", "lang": ""}, "reset_preset": True})
    check(f"{name}: no errors in the console", not errors, errors[:3])


if pw_ids("stream-mic"):
    sys.exit("another stream-mic is running (its virtual mic is there): stop it first")
CFG = os.path.join(W, "cfg")
os.makedirs(CFG)
MIC_TONE, PC_TONE = os.path.join(W, "mic.wav"), os.path.join(W, "pc.wav")
tone(MIC_TONE, MIC_HZ, 0.1)
tone(PC_TONE, PC_HZ, 0.05)
make_sink("smt-mic")
make_sink("smt-spk")
ENG = Engine(CFG)
MIC = Player("smt-mic", MIC_TONE)
PC = Player("stream-mic-pc-audio", PC_TONE)
time.sleep(1.0)
try:
    with sync_playwright() as pw:
        for name in BROWSERS:
            print(name, flush=True)
            if name == "chromium":
                b = pw.chromium.launch(args=["--use-fake-ui-for-media-stream", "--autoplay-policy=no-user-gesture-required"])
                ctx = b.new_context(permissions=["microphone"])
            else:
                b = pw.firefox.launch(firefox_user_prefs={"media.navigator.permission.disabled": True,
                                                          "media.navigator.streams.fake": False,
                                                          "media.autoplay.default": 0})
                ctx = b.new_context()
            page = ctx.new_page()
            try:
                page.goto(URL)
                capture_test(name, page)
            except Exception as e:
                check(f"{name}: capture ran", False, repr(e)[:300])
            try:
                page_test(name, page)
            except Exception as e:
                check(f"{name}: page test ran", False, repr(e)[:300])
                page.screenshot(path=os.path.join(W, name + "-failed.png"))
            b.close()
finally:
    MIC.stop()
    PC.stop()
    ENG.stop()
    remove_sink("smt-mic")
    remove_sink("smt-spk")
    bad = [r for r in results if not r[1]]
    print(f"\n{len(results) - len(bad)} passed, {len(bad)} failed" + "".join(f"\n  FAIL {n} ({d})" for n, _, d in bad))
    for f in os.listdir(W):
        if f.endswith(".wav"):
            os.remove(os.path.join(W, f))
    if bad:
        print(f"engine log: {os.path.join(W, 'engine.log')}")
    else:
        shutil.rmtree(W, ignore_errors=True)
