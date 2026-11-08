"""What the tests without a person share (func_test.py, browser_test.py): test devices on PipeWire (null sinks: one
as the mic, one as the headphones), tones, players and captures, the engine started on its own settings folder, and
its API. Call init(BINARY) first."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

import numpy as np
import soundfile as sf

WIN = False
BIN = None
PORT = 8795
R = 48000
W = tempfile.mkdtemp(prefix="smt-")
MIC_HZ, PC_HZ = 440.0, 1000.0
results = []


# ---- devices, sounds, captures

def pw_ids(name):
    out = subprocess.run(["pw-cli", "ls", "Node"], capture_output=True, text=True).stdout
    ids, cur = [], None
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("id "):
            cur = int(line.split()[1].rstrip(","))
        elif line.startswith("node.name") and line.split("=", 1)[1].strip().strip('"') == name:
            ids.append(cur)
    return ids


def make_sink(name):
    if not pw_ids(name):
        subprocess.run(["pw-cli", "create-node", "adapter",
                        "{ factory.name=support.null-audio-sink node.name=%s node.description=%s media.class=Audio/Sink "
                        "audio.position=[FL FR] object.linger=true monitor.channel-volumes=true }" % (name, name)],
                       capture_output=True, check=True)
        for _ in range(50):
            if pw_ids(name):
                break
            time.sleep(0.1)


def remove_sink(name):
    for i in pw_ids(name):
        subprocess.run(["pw-cli", "destroy", str(i)], capture_output=True)
    for _ in range(50):
        if not pw_ids(name):
            break
        time.sleep(0.1)


def tone(path, hz, amp, secs=420):
    t = np.arange(int(secs * R)) / R
    x = (amp * np.sin(2 * np.pi * hz * t)).astype(np.float32)
    sf.write(path, np.stack([x, x], 1), R, subtype="PCM_24")


def bursts(path, hz, amp, period=0.5, on=0.05, secs=60):
    """tone bursts every `period` s: for timing"""
    t = np.arange(int(secs * R)) / R
    x = (amp * np.sin(2 * np.pi * hz * t) * ((t % period) < on)).astype(np.float32)
    sf.write(path, np.stack([x, x], 1), R, subtype="PCM_24")


class Player:
    def __init__(self, target, path):
        self.p = subprocess.Popen(["pw-play", "--target", target, path], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def stop(self):
        self.p.terminate()
        self.p.wait()


def capture(dev, secs):
    """seconds of a PulseAudio source (a node, or SINK.monitor) as (n, 2)"""
    f = os.path.join(W, "cap.wav")
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "pulse", "-sample_rate", str(R), "-channels", "2",
                    "-i", dev, "-t", str(secs), "-c:a", "pcm_f32le", f], check=True, stdin=subprocess.DEVNULL)
    x, _ = sf.read(f, dtype="float64", always_2d=True)
    return x[int(0.3 * R):]  # PulseAudio's first moments


class capture_async:
    """a capture running while the test does something else: .wait() gives it"""
    def __init__(self, dev, secs):
        self.f = os.path.join(W, "acap.wav")
        self.p = subprocess.Popen(["ffmpeg", "-loglevel", "error", "-y", "-f", "pulse", "-sample_rate", str(R), "-channels", "2",
                                   "-i", dev, "-t", str(secs), "-c:a", "pcm_f32le", self.f], stdin=subprocess.DEVNULL)

    def wait(self):
        self.p.wait()
        x, _ = sf.read(self.f, dtype="float64", always_2d=True)
        return x


def vm_ids():
    """the virtual mic's and 電腦聲音's nodes (OBS loses a virtual mic that is made again)"""
    return pw_ids("stream-mic"), pw_ids("stream-mic-pc-audio")


def settings_read(cfg_home):
    return json.load(open(os.path.join(cfg_home, "stream-mic", "settings.json")))


def settings_break(cfg_home):
    """keep the settings file aside and put a broken one in its place"""
    os.rename(os.path.join(cfg_home, "stream-mic", "settings.json"), os.path.join(W, "settings.bak"))
    with open(os.path.join(cfg_home, "stream-mic", "settings.json"), "w") as f:
        f.write("{ broken")


def settings_restore(cfg_home):
    os.replace(os.path.join(W, "settings.bak"), os.path.join(cfg_home, "stream-mic", "settings.json"))


def level(x, hz):
    """dBFS of the `hz` component (amplitude of the sine, projected at that frequency under a Hann window, so the
    other tone does not leak in), mean of the channels"""
    m = x.mean(1)
    if len(m) < R // 4:
        return -200.0
    t = np.arange(len(m)) / R
    w = np.hanning(len(m))
    c, s = np.cos(2 * np.pi * hz * t), np.sin(2 * np.pi * hz * t)
    a = np.hypot(2 * np.sum(w * m * c), 2 * np.sum(w * m * s)) / np.sum(w)
    return 20 * np.log10(a + 1e-12)


# ---- the engine and its API

class Engine:
    def __init__(self, cfg_home, extra=(), test_pc=True):
        env = dict(os.environ, XDG_CONFIG_HOME=cfg_home)
        self.log = open(os.path.join(W, "engine.log"), "a")
        self.p = subprocess.Popen([BIN, "--mic", "smt-mic", "--out", "smt-spk", "--port", str(PORT), *extra],
                                  env=env, stdout=self.log, stderr=self.log, stdin=subprocess.DEVNULL)
        for _ in range(100):
            try:
                api("GET", "/api/state")
                if pw_ids("stream-mic"):
                    break
            except OSError:
                pass
            time.sleep(0.1)
        time.sleep(0.5)

    def running(self):
        return self.p.poll() is None

    def stop(self):
        self.p.terminate()
        try:
            self.p.wait(5)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait()
        for _ in range(50):
            if not pw_ids("stream-mic"):
                break
            time.sleep(0.1)


def api(method, path, body=None):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}{path}", method=method,
                                 data=None if body is None else json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=5) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def post(body):
    code, v = api("POST", "/api/state", body)
    assert code == 200, f"POST {body}: {code} {v}"
    time.sleep(0.4)
    return v


def check(name, ok, detail=""):
    results.append((name, bool(ok), detail))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"  ({detail})" if detail else ""), flush=True)


TESTS = []


def test(fn):
    TESTS.append(fn)
    return fn




def init(binary, port=8795):
    global BIN, PORT
    BIN, PORT = binary, port
