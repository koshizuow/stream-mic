"""smt_common for Windows: the same names, the engine and the sounds on a Windows machine over ssh (SMT_WIN=host).

the Windows test PC has one VB-CABLE and no loop to feed a mic, so the engine runs in its test mode:
  mic            "CABLE Output"       fed by --play to "CABLE Input"           (the name smt-mic)
  headphones     "Playback"           heard by --capture of its loopback       (smt-spk, smt-spk.monitor)
  virtual mic    "Virtual Output A"   STREAM_MIC_TEST_CABLE, heard the same way (stream-mic)
  computer sound "Virtual Output B"   STREAM_MIC_TEST_PC: the engine takes that output's loopback instead of the
                                      process loopback, so the mic's player is not in it (stream-mic-pc-audio)
The API through an ssh tunnel. Captures come at the device's rate (44.1 kHz on the Revelator) and are converted to
R here. The real process loopback is tested apart (func_test.py, process_loopback_hears_programs).
Call init(REMOTE_BINARY) first: a path on the Windows machine, relative to the user's folder."""
import atexit
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.request

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

WIN = True
HOST = os.environ["SMT_WIN"]  # the Windows test machine, an ssh host name
BIN = None
PORT = 8795
R = 48000
W = tempfile.mkdtemp(prefix="smt-")
RW = "smt-run"  # the remote work folder, under the binary's folder
MIC_HZ, PC_HZ = 440.0, 1000.0
results = []
DEV = {"smt-mic": "CABLE Input", "smt-spk": "Playback", "smt-spk.monitor": "Playback", "stream-mic": "Virtual Output A",
       "stream-mic-pc-audio": "Virtual Output B"}
MIC_IN = "CABLE Output"
# SMT_WIN_RIG=revelator: the interface's own loop instead, one clock throughout (set through Universal Control's
# service with io44-control's tools/ucctl.py, put back after): the test plays to Playback, Playback is sent to Stream Mix A (the only
# send to it, the inputs' sends at 0), the engine takes Stream Mix A as the mic; headphones on Virtual Output B, the
# virtual mic on Virtual Output A, the computer sound from VB-CABLE's loopback; Main muted (no tones in the speakers)
# Measured 2026-11-08: at 44.1 kHz the Revelator's Stream Mix A drops 32 samples now and then (at 48 kHz it is sample
# exact: `ucctl.py HOST setup 48000`), and the engine's resampler takes about 30 s to settle from some 900 ppm
# off; the headphones-missing test falls back to the default output (Playback) and loops. The standard rig is "cable".
RIG = os.environ.get("SMT_WIN_RIG", "cable")
UC_HOST = os.environ.get("SMT_UC_HOST", "")  # its address on the LAN, for the Universal Control service
# ucctl.py lives in the io44-control project (Universal Control's service, for the Revelator io44 rig)
UCCTL = os.environ.get("SMT_UCCTL", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "io44-control", "tools", "ucctl.py"))
RIG_SET = {"return/ch1/aux1": 0.7350000143051147, "line/ch1/aux1": 0.0, "line/ch2/aux1": 0.0, "line/ch3/aux1": 0.0,
           "return/ch2/aux1": 0.0, "return/ch3/aux1": 0.0, "fxreturn/ch1/aux1": 0.0, "main/ch1/mute": 1.0}
_rig_before = {}
if RIG == "revelator":
    DEV = {"smt-mic": "Playback", "smt-spk": "Virtual Output B", "smt-spk.monitor": "Virtual Output B",
           "stream-mic": "Virtual Output A", "stream-mic-pc-audio": "CABLE Input"}
    MIC_IN = "Stream Mix A"


def ucctl(*args):
    r = subprocess.run([sys.executable, UCCTL, UC_HOST, *args],
                       capture_output=True, text=True, timeout=30)
    if r.returncode != 0:
        raise RuntimeError(f"ucctl {args}: {r.stderr.strip()[:300]}")
    return r.stdout


def rig_up():
    for line in ucctl("get", *RIG_SET).splitlines():
        n, v = line.split(" ", 1)
        _rig_before[n] = float(v)
    ucctl("set", *(f"{n}={v}" for n, v in RIG_SET.items()))


def rig_down():
    if _rig_before:
        ucctl("set", *(f"{n}={v}" for n, v in _rig_before.items()))
        _rig_before.clear()
_tunnel = None
_sent = {}


def ps(cmd, check=True, timeout=60):
    """a PowerShell command on the Windows machine, in the binary's folder; its output"""
    full = "$ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; cd $env:USERPROFILE\\%s; %s" % (os.path.dirname(BIN).replace("/", "\\"), cmd)
    r = subprocess.run(["ssh", HOST, full], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"{cmd}: {r.stderr.strip()[:300]}")
    return r.stdout.strip()


def exe():
    return ".\\" + os.path.basename(BIN)


def send(path):
    """a local file to the remote work folder (once per file); its remote path"""
    name = os.path.basename(path)
    st = os.stat(path).st_mtime
    if _sent.get(name) != st:
        subprocess.run(["scp", "-q", path, f"{HOST}:{os.path.dirname(BIN)}/{RW}/{name}"], check=True)
        _sent[name] = st
    return f"{RW}\\{name}"


# ---- devices, sounds, captures

def pw_ids(name):
    """Windows: the virtual mic is a device that is always there; [] for anything else"""
    return [name] if name in ("stream-mic", "stream-mic-pc-audio") and ENGINE_UP[0] else []


ENGINE_UP = [False]


def vm_ids():
    return pw_ids("stream-mic"), pw_ids("stream-mic-pc-audio")


def make_sink(name):
    pass


def remove_sink(name):
    raise NotImplementedError("Windows: devices cannot be made or removed")


def tone(path, hz, amp, secs=10):
    """10 s, played looped: whole periods for 440 and 1000 Hz, float (the --play reader takes 16-bit or float)"""
    t = np.arange(int(secs * R)) / R
    x = (amp * np.sin(2 * np.pi * hz * t)).astype(np.float32)
    sf.write(path, np.stack([x, x], 1), R, subtype="FLOAT")


def bursts(path, hz, amp, period=0.5, on=0.05, secs=10):
    t = np.arange(int(secs * R)) / R
    x = (amp * np.sin(2 * np.pi * hz * t) * ((t % period) < on)).astype(np.float32)
    sf.write(path, np.stack([x, x], 1), R, subtype="FLOAT")


def spawn(cmdline, env=None):
    """a program left running after this ssh session (Windows OpenSSH ends the session's own processes with it):
    started by WMI through cmd, which sets `env` (None: remove the name) and redirects; the pid of that cmd"""
    sets = "".join(f'set "{k}=" & ' if v is None else f'set "{k}={v}" & ' for k, v in (env or {}).items())
    cmd = f'cmd /c "{sets}{cmdline}"'
    return ps("$d=(Get-Location).Path; (Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments "
              f"@{{CommandLine='{cmd}'; CurrentDirectory=$d}}).ProcessId")


def kill(pid):
    """the cmd and what it started"""
    ps(f"taskkill /T /F /PID {pid} 2>$null | Out-Null", check=False)


class Player:
    def __init__(self, target, path):
        rp = send(path)
        self.pid = spawn(f'{exe()} --play "{DEV.get(target, target)}" {rp} 3600')

    def stop(self):
        kill(self.pid)


def _fetch(rname, secs):
    f = os.path.join(W, rname)
    subprocess.run(["scp", "-q", f"{HOST}:{os.path.dirname(BIN)}/{RW}/{rname}", f], check=True)
    x, rate = sf.read(f, dtype="float64", always_2d=True)
    if rate != R:
        g = np.gcd(R, rate)
        x = resample_poly(x, R // g, rate // g, axis=0)
    return x


def capture(dev, secs):
    """seconds of an output (its loopback) or an input as (n, 2) at R"""
    ps(f'{exe()} --capture "{DEV.get(dev, dev)}" {secs + 0.3} {RW}\\cap.wav 2>$null', timeout=secs + 30)
    return _fetch("cap.wav", secs)[int(0.3 * R):]


class capture_async:
    """a capture running while the test does something else: .wait() gives it"""
    def __init__(self, dev, secs):
        self.secs = secs
        full = "cd $env:USERPROFILE\\%s; %s --capture \"%s\" %s %s\\acap.wav 2>$null" % (
            os.path.dirname(BIN).replace("/", "\\"), exe(), DEV.get(dev, dev), secs, RW)
        self.p = subprocess.Popen(["ssh", HOST, full], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def wait(self):
        self.p.wait()
        return _fetch("acap.wav", self.secs)


def _band(m, hz):
    """the power within 0.5 % of hz (a Hann window, the bins summed), as a sine's amplitude"""
    w = np.hanning(len(m))
    X = np.fft.rfft(m * w)
    f = np.fft.rfftfreq(len(m), 1 / R)
    band = np.abs(f - hz) <= max(hz * 5e-3, 3 * R / len(m))
    return np.sqrt(2 * np.sum(np.abs(X[band]) ** 2) / (len(m) * np.sum(w ** 2))) * np.sqrt(2)


def level(x, hz):
    """dBFS of the `hz` component, as smt_common.level, but the tone may come out a little off and wandering: the
    test mic (VB-CABLE) runs on the system's clock and the outputs on the Revelator's, about 0.1 % apart, and the
    engine follows the output's clock (its ratio moves by some 0.04 %). The power in a band of 0.5 % around it"""
    m = x.mean(1)
    if len(m) < R // 4:
        return -200.0
    return 20 * np.log10(_band(m, hz) + 1e-12)


# ---- the engine, its settings file and its API

class Engine:
    def __init__(self, cfg_home, extra=(), test_pc=True, desktop=False):
        # cfg_home names the remote settings folder (its last part, under the work folder). desktop: as started from
        # the Start menu, no XDG_CONFIG_HOME and no HOME (only ssh sessions have one), APPDATA = that folder
        self.cfg = f"{RW}\\{os.path.basename(cfg_home)}"
        ps(f"New-Item -ItemType Directory -Force {self.cfg} | Out-Null", check=False)
        full = ps(f"(Resolve-Path {self.cfg}).Path")
        env = {"STREAM_MIC_TEST_CABLE": DEV["stream-mic"], "STREAM_MIC_TEST_PC": DEV["stream-mic-pc-audio"] if test_pc else None}
        if desktop:
            env.update(XDG_CONFIG_HOME=None, HOME=None, APPDATA=full)
        else:
            env.update(XDG_CONFIG_HOME=full)
        args = " ".join(f'"{a}"' for a in ["--mic", MIC_IN, "--out", DEV["smt-spk"], "--port", str(PORT), *extra])
        self.pid = spawn(f"{exe()} {args} > {RW}\\engine.out 2>> {RW}\\engine.log", env)
        for _ in range(100):
            try:
                api("GET", "/api/state")
                break
            except OSError:
                time.sleep(0.1)
        ENGINE_UP[0] = True
        # the outputs' clocks are followed from the start: a few seconds to settle (pc_ppm, send_ppm in /api/stats)
        time.sleep(4.0)

    def running(self):
        return ps(f"[bool](Get-CimInstance Win32_Process -Filter 'ParentProcessId={self.pid}' | ? Name -eq '{os.path.basename(BIN)}')") == "True"

    def stop(self):
        kill(self.pid)
        time.sleep(0.5)
        ENGINE_UP[0] = False


def settings_read(cfg_home):
    """the file itself (copied here: a console between would re-encode it)"""
    f = os.path.join(W, "settings.json")
    subprocess.run(["scp", "-q", f"{HOST}:{os.path.dirname(BIN)}/{RW}/{os.path.basename(cfg_home)}/stream-mic/settings.json", f], check=True)
    return json.load(open(f, encoding="utf-8"))


def settings_break(cfg_home):
    """keep the settings file aside and put a broken one in its place"""
    d = f"{RW}\\{os.path.basename(cfg_home)}\\stream-mic"
    ps(f"Move-Item -Force {d}\\settings.json {RW}\\settings.bak; Set-Content -NoNewline -Path {d}\\settings.json -Value '{{ broken'")


def settings_restore(cfg_home):
    ps(f"Move-Item -Force {RW}\\settings.bak {RW}\\{os.path.basename(cfg_home)}\\stream-mic\\settings.json")


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
    time.sleep(0.6)
    return v


def check(name, ok, detail=""):
    results.append((name, bool(ok), detail))
    print(("  ok    " if ok else "  FAIL  ") + name + (f"  ({detail})" if detail else ""), flush=True)


TESTS = []


def test(fn):
    TESTS.append(fn)
    return fn


def cleanup():
    """the programs left (each under a cmd whose command line names the work folder), the work folder, the tunnel"""
    if BIN:
        ps("Get-CimInstance Win32_Process -Filter \"Name='cmd.exe'\" | ? { $_.CommandLine -like '*%s*' } | "
           "ForEach-Object { taskkill /T /F /PID $_.ProcessId 2>$null | Out-Null }; Start-Sleep 1; "
           "Remove-Item -Recurse -Force %s -ErrorAction SilentlyContinue" % (RW, RW), check=False)
    if _tunnel:
        _tunnel.terminate()
    rig_down()


def init(binary, port=8795):
    global BIN, PORT, _tunnel
    BIN, PORT = binary, port
    if ps(f"[bool](Get-Process {os.path.splitext(os.path.basename(BIN))[0]} -ErrorAction SilentlyContinue)") == "True":
        raise SystemExit(f"a {os.path.basename(BIN)} is running on {HOST}: stop it first")
    ps(f"Remove-Item -Recurse -Force {RW} -ErrorAction SilentlyContinue; New-Item -ItemType Directory {RW} | Out-Null")
    _tunnel = subprocess.Popen(["ssh", "-N", "-o", "ExitOnForwardFailure=yes", "-L", f"{PORT}:127.0.0.1:{PORT}", HOST],
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    atexit.register(cleanup)
    if RIG == "revelator":
        rig_up()
    time.sleep(1.0)
