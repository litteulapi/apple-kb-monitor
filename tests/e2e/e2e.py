#!/usr/bin/env python3
"""End-to-end scenarios of the apihub-app window. Started by tests/e2e/run.sh
INSIDE a bubblewrap sandbox: fake /sys/class/hidraw, /sys/bus/hid,
/sys/class/power_supply and /dev/hidraw*, no rssi-helper, no pkexec, no
network need. Owns its X server (Xvfb) and two private D-Bus buses (session
+ "system" with a fake BlueZ/UPower). Nothing reaches the real keyboard.

Each scenario writes <out>/<scenario>/result.json (+ screenshots); the
summary is <out>/summary.json and the exit code is 1 when one fails.
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from xprobe import Probe  # noqa: E402

TITLE = "Apple Keyboard Monitor"
WARMUP_S = 3.0


class Fail(Exception):
    pass


class Env:
    def __init__(self, a):
        self.a = a
        self.work = Path(a.work)
        self.out = Path(a.out)
        self.app = a.app
        self.daemon = a.daemon
        self.procs = []
        self.display = None
        self.base_env = None

    # ── infrastructure ──────────────────────────────────────────────────────
    def spawn(self, argv, log, env=None, **kw):
        f = open(log, "ab")
        kw.setdefault("stdout", f)
        p = subprocess.Popen(argv, stderr=f, env=env or self.base_env, start_new_session=True, **kw)
        self.procs.append(p)
        return p

    def start_xvfb(self):
        r, w = os.pipe()
        p = subprocess.Popen(["Xvfb", "-displayfd", str(w), "-screen", "0", "1920x1200x24", "-nolisten", "tcp",
                              "-noreset"], pass_fds=(w,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(w)
        self.procs.append(p)
        buf = b""
        deadline = time.time() + 15
        while time.time() < deadline and not buf.endswith(b"\n"):
            chunk = os.read(r, 16)
            if not chunk:
                break
            buf += chunk
        os.close(r)
        if not buf.strip():
            raise Fail("Xvfb did not start")
        self.display = ":" + buf.decode().strip()

    def start_bus(self, name):
        conf = self.work / f"{name}-bus.conf"
        conf.write_text(f"""<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:dir=/tmp</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy>
</busconfig>""")
        p = subprocess.Popen(["dbus-daemon", f"--config-file={conf}", "--nofork", "--print-address=1"],
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        return p.stdout.readline().decode().strip()

    def setup(self):
        self.start_xvfb()
        session = self.start_bus("session")
        system = self.start_bus("system")
        run = self.work / "runtime"
        run.mkdir(mode=0o700, exist_ok=True)
        env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS")}
        env.update({
            "DISPLAY": self.display, "DBUS_SESSION_BUS_ADDRESS": session, "DBUS_SYSTEM_BUS_ADDRESS": system,
            "XDG_RUNTIME_DIR": str(run), "XDG_SESSION_TYPE": "x11", "RUST_BACKTRACE": "1",
            "LIBGL_ALWAYS_SOFTWARE": "1", "NO_AT_BRIDGE": "1",
        })
        self.base_env = env
        sysspy = self.out / "bluez-spy.jsonl"
        p = self.spawn([sys.executable, str(HERE / "fakes.py"), "system", "--spy", str(sysspy)],
                       self.out / "fakes-system.log", stdout=subprocess.PIPE)
        p.stdout.readline()
        self.probe = Probe(self.display)
        self.base_n = len(self.procs)

    def teardown(self):
        for p in reversed(self.procs):
            if p.poll() is None:
                try:
                    os.killpg(p.pid, signal.SIGKILL)
                except OSError:
                    p.kill()
        for p in self.procs:
            try:
                p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass

    # ── per scenario helpers ────────────────────────────────────────────────
    def scenario_env(self, sc, history=None):
        d = self.out / sc
        d.mkdir(parents=True, exist_ok=True)
        home = self.work / "home" / sc
        if home.exists():
            shutil.rmtree(home)
        for sub in ("config", "state/apple-kb-monitor", "data", "cache"):
            (home / sub).mkdir(parents=True, exist_ok=True)
        if history:
            shutil.copy(history, home / "state/apple-kb-monitor/history.jsonl")
        env = dict(self.base_env)
        env.update({"HOME": str(home), "XDG_CONFIG_HOME": str(home / "config"),
                    "XDG_STATE_HOME": str(home / "state"), "XDG_DATA_HOME": str(home / "data"),
                    "XDG_CACHE_HOME": str(home / "cache")})
        return d, env

    def fake_daemon(self, d, env, mode="normal", history=None, **kw):
        argv = [sys.executable, str(HERE / "fakes.py"), "daemon", "--spy", str(d / "daemon-spy.jsonl"), "--mode", mode]
        if history:
            argv += ["--history", str(history)]
        for k, v in kw.items():
            argv += ["--" + k.replace("_", "-"), str(v)]
        p = self.spawn(argv, d / "fake-daemon.log", env=env, stdout=subprocess.PIPE)
        if p.stdout.readline().strip() != b"ready":
            raise Fail(f"fake daemon ({mode}) did not start, see {d / 'fake-daemon.log'}")
        return p

    def fake_desktop(self, d, env, delay=0.0):
        p = self.spawn([sys.executable, str(HERE / "fakes.py"), "desktop", "--spy", str(d / "desktop-spy.jsonl"),
                        "--delay", str(delay)], d / "fake-desktop.log", env=env, stdout=subprocess.PIPE)
        if p.stdout.readline().strip() != b"ready":
            raise Fail(f"fake desktop did not start, see {d / 'fake-desktop.log'}")
        return p

    def launch(self, d, env, strace=None, tag="app"):
        argv = [self.app]
        if strace:
            argv = ["strace", "-f", "-qq", "-y", "-o", str(strace), "-e",
                    "trace=open,openat,ioctl,read,write,writev,pwrite64",
                    "-P", "/dev/hidraw7", "-P", "/dev/hidraw3"] + argv
        return self.spawn(argv, d / f"{tag}.log", env=env)

    def windows(self):
        r = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", TITLE], env=self.base_env,
                           capture_output=True, text=True)
        return [int(w) for w in r.stdout.split()]

    def wait_window(self, p, timeout=30.0, d=None):
        t0 = time.time()
        while time.time() - t0 < timeout:
            ws = self.windows()
            if ws:
                return ws[0]
            if p.poll() is not None:
                raise Fail(f"apihub-app exited (rc={p.returncode}) before showing its window")
            time.sleep(0.1)
        shot = self.screenshot("root", self.out / f"no-window-{int(time.time())}.png")
        if d is not None:
            try:
                check_no_fallback(d)
            except Fail as ex:
                raise Fail(f"no window after {timeout:.0f} s AND {ex}. Screen capture: {shot}")
        raise Fail(f"no window titled '{TITLE}' after {timeout:.0f} s: start-up blocked (a synchronous call "
                   f"before the first frame?). Screen capture: {shot}")

    def screenshot(self, win, path):
        r = subprocess.run(["import", "-display", self.display, "-window", str(win), str(path)],
                           capture_output=True, text=True, timeout=30)
        return path if r.returncode == 0 else None

    def measure(self, d, p, win, seconds, poke=True):
        """Ping the window every 100 ms for `seconds`; returns latency stats."""
        stall_shot = None
        lat, misses, slow = [], 0, []
        t0 = time.time()
        n = 0
        while time.time() - t0 < seconds:
            if p.poll() is not None:
                raise Fail(f"apihub-app died during the measure (rc={p.returncode}), log: {d / 'app.log'}")
            if poke and n % 10 == 0:
                # Hover moves = repaints, as a user would cause.
                subprocess.run(["xdotool", "mousemove", "--window", str(win), str(50 + (n * 37) % 600),
                                str(60 + (n * 53) % 400)], env=self.base_env, capture_output=True)
            ms = self.probe.ping(win, timeout=5.0)
            el = time.time() - t0
            if (ms is None or (ms > self.a.max_latency_ms and el > WARMUP_S)) and stall_shot is None:
                # Capture at the moment of the stall, not at the end.
                stall_shot = self.screenshot(win, d / f"stall-t{el:.0f}s.png")
                misses += 1
                slow.append((round(el, 1), None))
            else:
                if el > WARMUP_S:
                    lat.append(ms)
                if ms > self.a.max_latency_ms and el > WARMUP_S:
                    slow.append((round(el, 1), round(ms, 1)))
            n += 1
            time.sleep(0.1)
        lat.sort()
        stats = {
            "pings": n, "no_answer_5s": misses, "max_ms": round(lat[-1], 1) if lat else None,
            "p50_ms": round(lat[len(lat) // 2], 1) if lat else None,
            "p99_ms": round(lat[int(len(lat) * 0.99) - 1], 1) if len(lat) > 1 else None,
            "over_threshold": slow[:20], "threshold_ms": self.a.max_latency_ms,
            "stall_capture": str(stall_shot) if stall_shot else None,
        }
        hb = self.work / "runtime/apple-kb-monitor/ui-heartbeat.json"
        if hb.exists():
            try:
                stats["heartbeat"] = json.loads(hb.read_text())
            except ValueError:
                stats["heartbeat"] = "unreadable"
        return stats

    def assert_responsive(self, stats, what, shot=None):
        shot = stats.get("stall_capture") or shot
        if stats["no_answer_5s"]:
            raise Fail(f"{what}: window NOT RESPONDING ({stats['no_answer_5s']} ping(s) without answer within 5 s) "
                       f"- this is the 'Ne répond plus' of the desktop. Capture: {shot}")
        if stats["over_threshold"]:
            worst = max(ms for _, ms in stats["over_threshold"] if ms is not None)
            raise Fail(f"{what}: {len(stats['over_threshold'])} event-loop stall(s) > {stats['threshold_ms']} ms "
                       f"(worst {worst:.0f} ms, at t={stats['over_threshold'][0][0]} s). Capture: {shot}")

    def close_and_check(self, d, p, win, label):
        self.probe.close_window(win)
        try:
            p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            raise Fail(f"{label}: apihub-app still running 10 s after WM_DELETE_WINDOW (window close ignored or hang)")
        time.sleep(2.0)
        left = leftover("apihub-app")
        if left:
            raise Fail(f"{label}: process leak after close: apihub-app pid(s) {left} still alive")
        if self.windows():
            raise Fail(f"{label}: the window REOPENED after close")
        return p.returncode

    def kill(self, p):
        if p and p.poll() is None:
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except OSError:
                pass
            p.wait(timeout=10)


def leftover(comm):
    out = []
    for e in Path("/proc").iterdir():
        if e.name.isdigit():
            try:
                if (e / "comm").read_text().strip() == comm and (e / "stat").read_text().split()[2] != "Z":
                    out.append(int(e.name))
            except OSError:
                pass
    return out


def check_no_fallback(d, tag="app"):
    """With a daemon on the bus the window must never read the keyboard
    itself (#199): a slow bus must not be mistaken for an absent daemon."""
    log = (d / f"{tag}.log").read_text(errors="replace") if (d / f"{tag}.log").exists() else ""
    if "daemon absent: local acquisition" in log:
        raise Fail("the window took the daemon for absent and started its OWN keyboard acquisition next to it "
                   f"(#199 violated: two readers of /dev/hidraw), log: {d / (tag + '.log')}")


def check_log(d, tag="app"):
    log = (d / f"{tag}.log").read_text(errors="replace") if (d / f"{tag}.log").exists() else ""
    m = re.search(r"panicked at[^\n]*\n?[^\n]*", log)
    if m:
        raise Fail(f"{tag} panicked: {m.group(0).strip()[:300]}")


# ── scenarios ──────────────────────────────────────────────────────────────

def sc_responsive(env: Env):
    """(a) realistic data from the daemon, 60 s of pings + hover."""
    d, e = env.scenario_env("responsive")
    env.fake_daemon(d, e, "normal", history=env.a.seed_history)
    p = env.launch(d, e)
    win = env.wait_window(p)
    stats = env.measure(d, p, win, env.a.duration)
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    check_no_fallback(d)
    env.assert_responsive(stats, "responsive (daemon normal)", shot)
    env.kill(p)
    return stats


def sc_open_close(env: Env):
    """(b) open/close 10 times: exit 0, no leftover process, no reopening."""
    d, e = env.scenario_env("open_close")
    env.fake_daemon(d, e, "normal")
    rcs = []
    for i in range(env.a.cycles):
        p = env.launch(d, e, tag=f"app{i}")
        win = env.wait_window(p)
        ms = env.probe.ping(win, timeout=5.0)
        if ms is None:
            raise Fail(f"cycle {i}: window not responding after opening")
        rc = env.close_and_check(d, p, win, f"cycle {i + 1}/{env.a.cycles}")
        check_log(d, f"app{i}")
        if rc != 0:
            raise Fail(f"cycle {i + 1}: apihub-app exited with {rc} after a normal close")
        rcs.append(rc)
    return {"cycles": len(rcs)}


def sc_daemon_absent(env: Env):
    """(c1) no daemon: local fallback on the fake hidraw; must stay responsive."""
    d, e = env.scenario_env("daemon_absent", history=env.a.seed_history)
    p = env.launch(d, e)
    win = env.wait_window(p)
    stats = env.measure(d, p, win, min(env.a.duration, 20))
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    env.assert_responsive(stats, "daemon absent", shot)
    env.kill(p)
    return stats


def sc_daemon_slow(env: Env):
    """(c2) daemon alive but answering after 3 s: the UI must not wait for it."""
    d, e = env.scenario_env("daemon_slow")
    env.fake_daemon(d, e, "slow", delay=3)
    p = env.launch(d, e)
    win = env.wait_window(p, timeout=45)
    stats = env.measure(d, p, win, min(env.a.duration, 25))
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    check_no_fallback(d)
    env.assert_responsive(stats, "daemon slow (3 s per call)", shot)
    env.kill(p)
    return stats


def sc_daemon_dies(env: Env):
    """(c3) daemon exits abruptly after 8 s: the UI survives and keeps answering."""
    d, e = env.scenario_env("daemon_dies")
    env.fake_daemon(d, e, "die", die_after=8)
    p = env.launch(d, e)
    win = env.wait_window(p)
    stats = env.measure(d, p, win, min(env.a.duration, 25))
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    env.assert_responsive(stats, "daemon died at t=8 s", shot)
    env.kill(p)
    return stats


def sc_daemon_garbage(env: Env):
    """(c4) daemon answering invalid JSON."""
    d, e = env.scenario_env("daemon_garbage")
    env.fake_daemon(d, e, "garbage")
    p = env.launch(d, e)
    win = env.wait_window(p)
    stats = env.measure(d, p, win, min(env.a.duration, 10))
    check_log(d)
    env.assert_responsive(stats, "daemon sends garbage", env.screenshot(win, d / "end.png"))
    env.kill(p)
    return stats


def sc_history_big(env: Env):
    """(d1) 50 000 real-shaped history points (daemon History + local file)."""
    d, e = env.scenario_env("history_50k", history=env.a.big_history)
    env.fake_daemon(d, e, "normal", history=env.a.big_history)
    p = env.launch(d, e)
    win = env.wait_window(p, timeout=45)
    stats = env.measure(d, p, win, min(env.a.duration, 30))
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    check_no_fallback(d)
    env.assert_responsive(stats, "history of 50 000 points", shot)
    env.kill(p)
    return stats


def sc_history_corrupt(env: Env):
    """(d2) corrupted local history (binary, invalid UTF-8, truncated), no daemon."""
    d, e = env.scenario_env("history_corrupt", history=env.a.bad_history)
    p = env.launch(d, e)
    win = env.wait_window(p)
    stats = env.measure(d, p, win, min(env.a.duration, 15))
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    env.assert_responsive(stats, "corrupted history", shot)
    env.kill(p)
    return stats


def sc_resize(env: Env):
    """(e) extreme sizes: tiny, narrow, huge; responsive and alive after each."""
    d, e = env.scenario_env("resize")
    env.fake_daemon(d, e, "normal", history=env.a.seed_history)
    p = env.launch(d, e)
    win = env.wait_window(p)
    res = {}
    for w, h in [(1, 1), (120, 90), (260, 900), (1900, 140), (1920, 1200), (640, 480)]:
        subprocess.run(["xdotool", "windowsize", str(win), str(w), str(h)], env=env.base_env, capture_output=True)
        time.sleep(1.0)
        worst = 0.0
        for _ in range(10):
            ms = env.probe.ping(win, timeout=5.0)
            if ms is None:
                raise Fail(f"resize {w}x{h}: window not responding. Capture: {env.screenshot(win, d / f'{w}x{h}.png')}")
            worst = max(worst, ms)
            time.sleep(0.05)
        env.screenshot(win, d / f"{w}x{h}.png")
        if p.poll() is not None:
            raise Fail(f"resize {w}x{h}: apihub-app died (rc={p.returncode})")
        if worst > env.a.max_latency_ms * 3:
            raise Fail(f"resize {w}x{h}: event-loop stall of {worst:.0f} ms. Capture: {d / f'{w}x{h}.png'}")
        res[f"{w}x{h}"] = round(worst, 1)
    check_log(d)
    check_no_fallback(d)
    env.kill(p)
    return {"worst_ping_ms_per_size": res}


def sc_desktop(env: Env):
    """(c5) a desktop whose portal and tray host answer at once (as on KDE)."""
    d, e = env.scenario_env("desktop")
    env.fake_desktop(d, e, 0)
    env.fake_daemon(d, e, "normal", history=env.a.seed_history)
    p = env.launch(d, e)
    win = env.wait_window(p)
    stats = env.measure(d, p, win, min(env.a.duration, 15))
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    check_no_fallback(d)
    env.assert_responsive(stats, "desktop services (portal, tray host)", shot)
    env.kill(p)
    return stats


def sc_desktop_slow(env: Env):
    """(c6) settings portal and tray host answering after 40 s (measured on
    dbus-broker: no reply timeout): the window must keep answering."""
    d, e = env.scenario_env("desktop_slow")
    env.fake_desktop(d, e, 40)
    env.fake_daemon(d, e, "normal", history=env.a.seed_history)
    p = env.launch(d, e)
    win = env.wait_window(p, timeout=90, d=d)
    stats = env.measure(d, p, win, min(env.a.duration, 30))
    shot = env.screenshot(win, d / "end.png")
    check_log(d)
    check_no_fallback(d)
    env.assert_responsive(stats, "slow desktop services (portal/tray host answer after 40 s)", shot)
    env.kill(p)
    return stats


HID_WRITES = re.compile(r"HIDIOCS(FEATURE|OUTPUT|INPUT)|HIDIOCSET")
SAFE_BLUEZ = {"GetManagedObjects", "Get", "GetAll", "Introspect", "EnumerateDevices",
              "RegisterBatteryProvider", "UnregisterBatteryProvider"}


def analyse_strace(path, who, may_open_kb):
    probs, gets = [], {}
    if not path.exists():
        return [f"{who}: no strace output"], gets
    for line in path.read_text(errors="replace").splitlines():
        if "/dev/hidraw3" in line:
            probs.append(f"{who} touched /dev/hidraw3 (a non-Apple HID device, outside the allow list): {line[:160]}")
        if "/dev/hidraw7" not in line:
            continue
        if not may_open_kb and re.search(r"\bopen(at)?\(", line):
            probs.append(f"{who} opened the keyboard hidraw while the daemon owns it (#199): {line[:160]}")
        if re.search(r"\b(write|writev|pwrite64)\(\d+</dev/hidraw7>", line):
            probs.append(f"{who} WROTE to the keyboard: {line[:160]}")
        if HID_WRITES.search(line):
            probs.append(f"{who} sent a SET report to the keyboard: {line[:160]}")
        m = re.search(r"ioctl\(\d+</dev/hidraw7>, (HIDIOC\w+(?:\(\d+\))?|0x[0-9a-f]+)", line)
        if m:
            gets[m.group(1)] = gets.get(m.group(1), 0) + 1
            if m.group(1).startswith("0x"):
                probs.append(f"{who}: unknown ioctl on the keyboard: {line[:160]}")
    return probs, gets


def sc_spy(env: Env):
    """(f) real daemon on the fake keyboard + window, both under strace:
    no write, no SET report, nothing outside the allow list, no pairing call."""
    d, e = env.scenario_env("spy", history=env.a.seed_history)
    kb = Path("/dev/hidraw7")
    size0 = kb.stat().st_size
    dstrace, astrace = d / "daemon.strace", d / "app.strace"
    dm = env.spawn(["strace", "-f", "-qq", "-y", "-o", str(dstrace), "-e", "trace=open,openat,ioctl,read,write,writev,pwrite64",
                    "-P", "/dev/hidraw7", "-P", "/dev/hidraw3", env.daemon, "--no-notify", "--no-connection-notify"],
                   d / "daemon.log", env=e)
    t0 = time.time()
    while time.time() - t0 < 20:
        r = subprocess.run(["busctl", "--user", "status", "com.agenceapi.AppleKbMonitor1"], env=e, capture_output=True)
        if r.returncode == 0:
            break
        if dm.poll() is not None:
            raise Fail(f"test daemon exited (rc={dm.returncode}), log {d / 'daemon.log'}")
        time.sleep(0.2)
    else:
        raise Fail("test daemon never took its bus name")
    p = env.launch(d, e, strace=astrace)
    win = env.wait_window(p, timeout=60)
    time.sleep(min(env.a.duration, 20))
    shot = env.screenshot(win, d / "end.png")
    alive = p.poll() is None and dm.poll() is None
    env.kill(p)
    env.kill(dm)
    check_log(d)
    check_log(d, "daemon")
    probs = []
    pa, ga = analyse_strace(astrace, "apihub-app", may_open_kb=False)
    pd, gd = analyse_strace(dstrace, "apple-kb-monitord", may_open_kb=True)
    probs += pa + pd
    if kb.stat().st_size != size0:
        probs.append(f"the fake keyboard node grew from {size0} to {kb.stat().st_size} bytes: something WROTE to it")
    spy = env.out / "bluez-spy.jsonl"
    calls = [json.loads(l) for l in spy.read_text().splitlines()] if spy.exists() else []
    bad = sorted({c["method"] for c in calls if c["method"] not in SAFE_BLUEZ})
    if bad:
        probs.append(f"BlueZ calls that change the keyboard/pairing: {bad}")
    if not alive:
        probs.append("a process died during the spy run")
    if probs:
        raise Fail("; ".join(probs[:6]) + f". Capture: {shot}")
    return {"daemon_ioctls": gd, "app_ioctls": ga, "bluez_methods": sorted({c["method"] for c in calls})}


def compare(ref, shot, diff_path, tol_pixels=0.03, tol_level=48):
    from PIL import Image, ImageChops
    a, b = Image.open(ref).convert("RGB"), Image.open(shot).convert("RGB")
    if a.size != b.size:
        return f"size {b.size} != reference {a.size}"
    diff = ImageChops.difference(a, b).convert("L").point(lambda v: 255 if v > tol_level else 0)
    changed = sum(diff.histogram()[255:]) / (a.size[0] * a.size[1])
    if changed > tol_pixels:
        overlay = Image.blend(b, Image.new("RGB", b.size, (255, 0, 0)), 0.0)
        overlay.paste((255, 0, 0), mask=diff)
        overlay.save(diff_path)
        return f"{changed * 100:.1f} % of pixels differ (> {tol_pixels * 100:.0f} %), diff: {diff_path}"
    return None


def blank(shot):
    from PIL import Image, ImageStat
    st = ImageStat.Stat(Image.open(shot).convert("L"))
    return st.stddev[0] < 3.0


def sc_screens(env: Env):
    """(g) fixed data, fixed sizes, compared with tests/e2e/refs/*.png."""
    d, e = env.scenario_env("screens", history=env.a.seed_history)
    env.fake_daemon(d, e, "static", history=env.a.seed_history)
    p = env.launch(d, e)
    win = env.wait_window(p)
    refs = HERE / "refs"
    probs, done = [], []
    for w, h in [(900, 700), (420, 700)]:
        subprocess.run(["xdotool", "windowsize", str(win), str(w), str(h)], env=env.base_env, capture_output=True)
        subprocess.run(["xdotool", "mousemove", "1900", "1190"], env=env.base_env, capture_output=True)
        time.sleep(2.5)
        env.probe.ping(win)
        name = f"main-{w}x{h}.png"
        shot = env.screenshot(win, d / name)
        if not shot:
            probs.append(f"no screenshot for {name}")
            continue
        if blank(shot):
            probs.append(f"{name}: the window is BLANK (uniform image): {shot}")
            continue
        ref = refs / name
        if env.a.update_refs:
            refs.mkdir(exist_ok=True)
            shutil.copy(shot, ref)
            done.append(f"{name}: reference updated")
        elif not ref.exists():
            done.append(f"{name}: no reference (tests/e2e/run.sh --update-refs)")
        else:
            why = compare(ref, shot, d / ("diff-" + name))
            if why:
                probs.append(f"{name} differs from the reference: {why}; capture: {shot}")
            else:
                done.append(f"{name}: matches")
    check_log(d)
    check_no_fallback(d)
    env.kill(p)
    if probs:
        raise Fail("; ".join(probs))
    return {"screens": done}


SCENARIOS = {
    "responsive": sc_responsive, "open_close": sc_open_close, "daemon_absent": sc_daemon_absent,
    "daemon_slow": sc_daemon_slow, "daemon_dies": sc_daemon_dies, "daemon_garbage": sc_daemon_garbage,
    "desktop": sc_desktop, "desktop_slow": sc_desktop_slow,
    "history_50k": sc_history_big, "history_corrupt": sc_history_corrupt, "resize": sc_resize,
    "spy": sc_spy, "screens": sc_screens,
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--app", required=True)
    ap.add_argument("--daemon", required=True)
    ap.add_argument("--seed-history", required=True)
    ap.add_argument("--big-history", required=True)
    ap.add_argument("--bad-history", required=True)
    ap.add_argument("--duration", type=float, default=60.0)
    ap.add_argument("--cycles", type=int, default=10)
    ap.add_argument("--max-latency-ms", type=float, default=100.0)
    ap.add_argument("--only", default="")
    ap.add_argument("--update-refs", action="store_true")
    a = ap.parse_args()
    # Safety: this script kills apihub-app processes it finds in /proc. It must
    # only ever see its own (private pid namespace + /proc of run.sh's bwrap).
    if os.environ.get("AKM_E2E") != "1" or Path("/proc/1/comm").read_text().strip() in ("systemd", "init"):
        sys.exit("e2e.py refuses to run outside the sandbox of tests/e2e/run.sh (private pid namespace)")
    env = Env(a)
    Path(a.out).mkdir(parents=True, exist_ok=True)
    results, failed = {}, []
    try:
        try:
            env.setup()
        except Fail as ex:
            print(f"e2e: environment setup FAILED: {ex}")
            (Path(a.out) / "summary.json").write_text(json.dumps({"ok": False, "setup_error": str(ex)}) + "\n")
            return 1
        for name, fn in SCENARIOS.items():
            if a.only and name not in a.only.split(","):
                continue
            t0 = time.time()
            print(f"  e2e {name:<16}", end=" ", flush=True)
            try:
                detail = fn(env)
                results[name] = {"ok": True, "seconds": round(time.time() - t0, 1), "detail": detail}
                print(f"ok   ({time.time() - t0:.0f}s) {json.dumps(detail)[:150]}", flush=True)
            except Fail as ex:
                results[name] = {"ok": False, "seconds": round(time.time() - t0, 1), "error": str(ex)}
                failed.append(name)
                print(f"FAIL ({time.time() - t0:.0f}s) {ex}", flush=True)
            # Kill whatever a scenario left (its app, its fake daemon).
            for p in env.procs[env.base_n:]:
                env.kill(p)
            del env.procs[env.base_n:]
            for pid in leftover("apihub-app"):
                try:
                    os.kill(pid, signal.SIGKILL)
                except OSError:
                    pass
            (Path(a.out) / name).mkdir(exist_ok=True)
            (Path(a.out) / name / "result.json").write_text(json.dumps(results[name], indent=2) + "\n")
    finally:
        env.teardown()
    summary = {"ok": not failed, "failed": failed, "scenarios": results}
    (Path(a.out) / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"e2e: {len(results) - len(failed)}/{len(results)} scenario(s) passed" + (f", FAILED: {', '.join(failed)}" if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
