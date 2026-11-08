# /// script
# dependencies = ["numpy", "soundfile", "dbus-next==0.2.3", "python-xlib==0.33"]
# ///
"""The app (engine, window and tray in one program) on Linux, without a person or a desktop: a virtual screen (Xvfb)
and a D-Bus session of its own, where this script plays the desktop's tray (org.kde.StatusNotifierWatcher). The app's
icon registers there; its menu is read and clicked over D-Bus (com.canonical.dbusmenu), the way a panel would.

Checks: started with --hidden it is in the tray only; 打開設定 shows the window; 原音 sets bypass, and the check mark
follows bypass set elsewhere (the page); closing the window keeps it running in the tray; a second start brings the
window back and exits; 結束 stops it and its virtual mic goes.

Needs Xvfb and dbus-run-session. Usage: dbus-run-session -- uv run tools/app_test.py APP_BINARY
"""
import asyncio
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import smt_common  # noqa: E402
from smt_common import MIC_HZ, W, Player, api, check, make_sink, pw_ids, remove_sink, results, tone  # noqa: E402

from dbus_next import Message, MessageType, Variant  # noqa: E402
from dbus_next.aio import MessageBus  # noqa: E402
from Xlib import X, display as xdisplay, protocol  # noqa: E402

APP = sys.argv[1]
smt_common.init(APP, port=8797)
DISP = ":79"
WATCHER = "org.kde.StatusNotifierWatcher"


class Tray:
    """the desktop's side: takes the item's registration, answers what the item asks of the watcher"""

    def __init__(self, bus):
        self.bus, self.items = bus, []
        bus.add_message_handler(self.handle)

    def handle(self, m):
        if m.message_type != MessageType.METHOD_CALL or m.path != "/StatusNotifierWatcher":
            return None
        if m.member == "RegisterStatusNotifierItem":
            arg = m.body[0]
            # a path: the item is the sender at that path; a bus name: its usual path
            self.items.append((m.sender, arg) if arg.startswith("/") else (arg, "/StatusNotifierItem"))
            return Message.new_method_return(m)
        if m.member == "RegisterStatusNotifierHost":
            return Message.new_method_return(m)
        props = {"IsStatusNotifierHostRegistered": Variant("b", True), "ProtocolVersion": Variant("i", 0),
                 "RegisteredStatusNotifierItems": Variant("as", [s + p for s, p in self.items])}
        if m.interface == "org.freedesktop.DBus.Properties" and m.member == "Get":
            return Message.new_method_return(m, "v", [props[m.body[1]]])
        if m.interface == "org.freedesktop.DBus.Properties" and m.member == "GetAll":
            return Message.new_method_return(m, "a{sv}", [props])
        if m.member == "Introspect":
            return Message.new_method_return(m, "s", ["<node/>"])
        return None


async def call(bus, dest, path, iface, member, sig="", body=()):
    r = await bus.call(Message(destination=dest, path=path, interface=iface, member=member, signature=sig, body=list(body)))
    if r.message_type == MessageType.ERROR:
        raise RuntimeError(f"{member}: {r.error_name} {r.body}")
    return r.body


def flat(layout, out=None):
    """dbusmenu's layout (id, props, children) as {label: (id, props)}"""
    out = {} if out is None else out
    i, props, kids = layout
    if "label" in props:
        out[props["label"].value.replace("_", "")] = (i, {k: v.value for k, v in props.items()})
    for k in kids:
        flat(k.value, out)
    return out


def window(d):
    """the app's main window: the largest named stream-mic (GTK also makes a 10x10 one by that name, never shown)"""
    found = []

    def walk(w):
        try:
            if w.get_wm_name() == "stream-mic":
                g = w.get_geometry()
                found.append((g.width * g.height, w))
            for c in w.query_tree().children:
                walk(c)
        except Exception:
            pass
    walk(d.screen().root)
    return max(found, key=lambda x: x[0])[1] if found else None


def viewable(d):
    w = window(d)
    return w is not None and w.get_attributes().map_state == X.IsViewable


def wait_for(f, secs=5.0):
    t = time.time()
    while time.time() - t < secs:
        v = f()
        if v:
            return v
        time.sleep(0.1)
    return f()


async def main():
    if pw_ids("stream-mic"):
        sys.exit("another stream-mic is running (its virtual mic is there): stop it first")
    if "DBUS_SESSION_BUS_ADDRESS" not in os.environ:
        sys.exit("run under dbus-run-session")
    xv = subprocess.Popen(["Xvfb", DISP, "-screen", "0", "1280x960x24"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(50):
        if os.path.exists(f"/tmp/.X11-unix/X{DISP[1:]}"):
            break
        time.sleep(0.1)
    time.sleep(0.3)
    bus = await MessageBus().connect()
    tray = Tray(bus)
    await call(bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName", "su", [WATCHER, 0])
    cfg = os.path.join(W, "cfg")
    os.makedirs(cfg)
    env = dict(os.environ, DISPLAY=DISP, XDG_CONFIG_HOME=cfg, LIBGL_ALWAYS_SOFTWARE="1")
    args = [APP, "--port", str(smt_common.PORT), "--mic", "smt-mic", "--out", "smt-spk"]
    make_sink("smt-mic")
    make_sink("smt-spk")
    mic_tone = os.path.join(W, "mic.wav")
    tone(mic_tone, MIC_HZ, 0.1, secs=120)
    mic = Player("smt-mic", mic_tone)
    log = open(os.path.join(W, "app.log"), "w")
    app = subprocess.Popen(args + ["--hidden"], env=env, stdout=log, stderr=log, stdin=subprocess.DEVNULL)
    d = xdisplay.Display(DISP)
    try:
        ok = wait_for(lambda: pw_ids("stream-mic"), 10)
        check("app starts, its virtual mic is there", ok)
        for _ in range(100):
            if tray.items:
                break
            await asyncio.sleep(0.1)
        check("the icon registers with the tray", tray.items, tray.items)
        if not tray.items:
            return
        svc, path = tray.items[0]
        item = await call(bus, svc, path, "org.freedesktop.DBus.Properties", "GetAll", "s", ["org.kde.StatusNotifierItem"])
        item = {k: v.value for k, v in item[0].items()}
        check("icon has a picture", bool(item.get("IconPixmap") or item.get("IconName")))
        check("icon's tooltip or title says stream-mic", "stream-mic" in str(item.get("Title", "")) + str(item.get("ToolTip", "")) + str(item.get("Id", "")),
              {k: item.get(k) for k in ("Id", "Title")})
        menu = item.get("Menu")
        check("icon has a menu", bool(menu), menu)

        async def layout():
            _, lay = await call(bus, svc, menu, "com.canonical.dbusmenu", "GetLayout", "iias", [0, -1, []])
            return flat(lay)

        async def click(label):
            i, _ = (await layout())[label]
            await call(bus, svc, menu, "com.canonical.dbusmenu", "Event", "isvu", [i, "clicked", Variant("i", 0), int(time.time())])

        items = await layout()
        check("menu: 打開設定, 原音, 結束", {"打開設定", "原音（暫停處理）", "結束 stream-mic"} <= set(items), list(items))
        check("started with --hidden: no window on screen", not viewable(d))

        await click("打開設定")
        ok = wait_for(lambda: viewable(d))
        if not ok:
            print(subprocess.run(["xwininfo", "-root", "-tree", "-display", DISP], capture_output=True, text=True).stdout[:3000])
        check("打開設定 shows the window", ok)
        check("... the page in it talks to the engine (app: true)", api("GET", "/api/state")[1].get("app") is True)

        await click("原音（暫停處理）")
        check("原音 turns bypass on", wait_for(lambda: api("GET", "/api/state")[1]["bypass"] is True))
        await asyncio.sleep(0.6)
        check("... its check mark on", (await layout())["原音（暫停處理）"][1].get("toggle-state") == 1)
        api("POST", "/api/state", {"bypass": False})
        await asyncio.sleep(0.8)
        check("bypass off from the page: the check mark follows", (await layout())["原音（暫停處理）"][1].get("toggle-state") == 0)

        # the window's close button: WM_DELETE_WINDOW, as a window manager sends it
        w = window(d)
        proto, delete = d.intern_atom("WM_PROTOCOLS"), d.intern_atom("WM_DELETE_WINDOW")
        w.send_event(protocol.event.ClientMessage(window=w, client_type=proto, data=(32, [delete, X.CurrentTime, 0, 0, 0])))
        d.flush()
        check("closing the window hides it", wait_for(lambda: not viewable(d)))
        check("... and stream-mic keeps running", app.poll() is None and pw_ids("stream-mic"))

        t0 = time.time()
        second = subprocess.run(args, env=env, capture_output=True, timeout=20)
        check("a second start exits at once", second.returncode == 0 and time.time() - t0 < 5, f"{second.returncode}, {time.time() - t0:.2f} s")
        check("... and brings the window back", wait_for(lambda: viewable(d)))
        check("... still one virtual mic", len(pw_ids("stream-mic")) == 1)

        await click("結束 stream-mic")
        try:
            app.wait(10)
        except subprocess.TimeoutExpired:
            pass
        check("結束 stops it", app.poll() is not None, app.poll())
        check("... and its virtual mic goes", wait_for(lambda: not pw_ids("stream-mic")))
    finally:
        if app.poll() is None:
            app.terminate()
            app.wait(5)
        mic.stop()
        remove_sink("smt-mic")
        remove_sink("smt-spk")
        d.close()
        xv.terminate()
        bad = [r for r in results if not r[1]]
        print(f"\n{len(results) - len(bad)} passed, {len(bad)} failed" + "".join(f"\n  FAIL {n} ({dd})" for n, _, dd in bad))
        for f in os.listdir(W):
            if f.endswith(".wav"):
                os.remove(os.path.join(W, f))
        if bad:
            print(f"app log: {os.path.join(W, 'app.log')}")
        else:
            shutil.rmtree(W, ignore_errors=True)


asyncio.run(main())
