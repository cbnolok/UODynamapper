#!/usr/bin/env python3
"""Bounded exact-PID X11 preview on a named KWin desktop."""

import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time
import uuid


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
PNG_END = b"\x00\x00\x00\x00IEND\xaeB`\x82"


def png_complete(path):
    """Check write completion, not full image validity; decode/view afterward."""
    try:
        with Path(path).open("rb") as stream:
            if stream.read(8) != PNG_SIGNATURE:
                return False
            stream.seek(-len(PNG_END), os.SEEK_END)
            return stream.read() == PNG_END
    except (FileNotFoundError, OSError):
        return False


def stopped_child(process, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        pid, status = os.waitpid(process.pid, os.WNOHANG | os.WUNTRACED)
        if pid:
            if os.WIFSTOPPED(status) and os.WSTOPSIG(status) == signal.SIGSTOP:
                return
            if os.WIFEXITED(status) or os.WIFSIGNALED(status):
                process.returncode = os.waitstatus_to_exitcode(status)
                raise RuntimeError(f"Launch wrapper exited: {process.returncode}")
        time.sleep(0.01)
    raise RuntimeError("Launch wrapper did not stop before window creation")


def terminate_owned_child(process):
    if process is None or process.poll() is not None:
        return
    process.terminate()
    # A pending SIGTERM will not be handled until a stopped child is resumed.
    process.send_signal(signal.SIGCONT)
    try:
        process.wait(timeout=4)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=4)


def kwin_script(pid, desktop, service):
    return f"""const target = workspace.desktops.find(d => d.name === {json.dumps(desktop)});
if (target) {{
    workspace.windowAdded.connect(function(window) {{
        if (Number(window.pid) !== {int(pid)}) return;
        window.desktops = [target];
        window.demandsAttention = false;
        callDBus({json.dumps(service)}, '/Preview', 'org.codex.KdePreview', 'Placed',
            String(window.pid), JSON.stringify(window.desktops.map(d => d.name)));
    }});
}}
callDBus({json.dumps(service)}, '/Preview', 'org.codex.KdePreview', 'Ready', Boolean(target));
"""


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--desktop", required=True, help="Exact KWin desktop name")
    parser.add_argument("--cwd", required=True, type=Path, help="Isolated app working directory")
    parser.add_argument("--display", default=os.environ.get("DISPLAY"), help="X11 display (or inherited DISPLAY)")
    parser.add_argument("--capture-env", help="App-native PNG output environment variable")
    parser.add_argument("--placement-timeout", type=float, default=8)
    parser.add_argument("--capture-timeout", type=float, default=30)
    parser.add_argument("--seconds", type=float, default=3, help="Preview duration when capture is not requested")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    if not args.command:
        parser.error("an executable command is required after --")
    if not args.display:
        parser.error("--display or DISPLAY is required")
    if not args.cwd.is_dir():
        parser.error("--cwd must be an existing isolated working directory")
    if not args.desktop:
        parser.error("--desktop must not be empty")
    if args.capture_env and not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", args.capture_env):
        parser.error("--capture-env must be an environment variable name")
    if args.capture_env in {"HOME", "CODEX_HOME", "DISPLAY", "WAYLAND_DISPLAY"}:
        parser.error("--capture-env must be an application-specific output variable")
    for key in ["placement_timeout", "capture_timeout"]:
        value = getattr(args, key)
        if not 0 < value <= 300:
            parser.error(f"--{key.replace('_', '-')} must be greater than 0 and at most 300 seconds")
    if not 0 <= args.seconds <= 300:
        parser.error("--seconds must be between 0 and 300")
    return args


def main(argv=None):
    args = parse_args(argv)
    # Load GUI dependencies only after argument/help validation.
    import dbus
    import dbus.service
    from dbus.mainloop.glib import DBusGMainLoop
    from gi.repository import GLib

    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    bus.get_name_owner("org.kde.KWin")
    run_dir = Path(tempfile.mkdtemp(prefix="codex-kde-preview-"))
    capture = run_dir / "capture.png"
    service_name = f"org.codex.KdePreview.p{os.getpid()}.r{uuid.uuid4().hex}"
    bus_name = dbus.service.BusName(service_name, bus=bus, do_not_queue=True)
    ready = []
    placed = []

    class Callback(dbus.service.Object):
        @dbus.service.method("org.codex.KdePreview", in_signature="b", out_signature="")
        def Ready(self, exists):
            ready.append(bool(exists))

        @dbus.service.method("org.codex.KdePreview", in_signature="ss", out_signature="")
        def Placed(self, pid, desktops):
            placed.append((int(pid), json.loads(str(desktops))))

    callback = Callback(bus_name, "/Preview")
    context = GLib.MainContext.default()
    scripting = dbus.Interface(bus.get_object("org.kde.KWin", "/Scripting"), "org.kde.kwin.Scripting")
    child = None
    loaded = False
    plugin = f"codex-kde-preview-{uuid.uuid4().hex}"

    def pump():
        while context.pending():
            context.iteration(False)

    def wait_for(predicate, timeout, description):
        deadline = time.monotonic() + timeout
        while True:
            pump()
            if predicate():
                return
            if child.poll() is not None:
                raise RuntimeError(f"App exited before {description}: {child.returncode}; see {run_dir / 'app.log'}")
            if time.monotonic() >= deadline:
                raise RuntimeError(f"Timed out waiting for {description}; see {run_dir / 'app.log'}")
            time.sleep(0.05)

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    env = os.environ.copy()
    env.pop("WAYLAND_DISPLAY", None)
    env["DISPLAY"] = args.display
    if args.capture_env:
        env[args.capture_env] = str(capture)
    print(json.dumps({"artifacts": str(run_dir), "kwin_plugin": plugin}), flush=True)
    try:
        with (run_dir / "app.log").open("w") as log:
            # Arguments are passed as argv, never interpolated into shell code.
            child = subprocess.Popen(
                ["/bin/sh", "-c", 'kill -STOP "$$"; exec "$@"', "preview-wrapper", *args.command],
                cwd=args.cwd,
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
            stopped_child(child, args.placement_timeout)
            script = run_dir / "place-window.js"
            script.write_text(kwin_script(child.pid, args.desktop, service_name))
            script_id = int(scripting.loadScript(str(script), plugin, signature="ss"))
            if script_id < 0:
                raise RuntimeError("KWin rejected the preview script")
            loaded = True
            script_api = dbus.Interface(
                bus.get_object("org.kde.KWin", f"/Scripting/Script{script_id}"),
                "org.kde.kwin.Script",
            )
            script_api.run()
            wait_for(lambda: bool(ready), args.placement_timeout, "KWin desktop preflight")
            if ready != [True]:
                raise RuntimeError(f"KWin desktop {args.desktop!r} was not found; app remains stopped")
            child.send_signal(signal.SIGCONT)
            wait_for(lambda: bool(placed), args.placement_timeout, "exact-PID desktop assignment")
            if any(pid != child.pid or desktops != [args.desktop] for pid, desktops in placed):
                raise RuntimeError(f"Unexpected desktop placement: {placed!r}")
            print(json.dumps({"pid": child.pid, "desktop": args.desktop, "placement_verified": True}), flush=True)
            if args.capture_env:
                wait_for(lambda: png_complete(capture), args.capture_timeout, "complete app-produced PNG")
                print(json.dumps({"capture": str(capture)}), flush=True)
            else:
                deadline = time.monotonic() + args.seconds
                while child.poll() is None and time.monotonic() < deadline:
                    pump()
                    time.sleep(0.05)
    finally:
        cleanup_errors = []
        try:
            terminate_owned_child(child)
        except Exception as error:
            cleanup_errors.append(f"Owned child cleanup: {error}")
        if loaded:
            try:
                if not bool(scripting.unloadScript(plugin)):
                    cleanup_errors.append(f"KWin did not unload {plugin!r}")
            except Exception as error:
                cleanup_errors.append(f"KWin rule cleanup: {error}")
        callback.remove_from_connection()
        print(json.dumps({
            "cleanup_complete": not cleanup_errors,
            "cleanup_errors": cleanup_errors,
            "artifacts": str(run_dir),
        }), flush=True)
        if cleanup_errors:
            raise RuntimeError("; ".join(cleanup_errors))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
    except Exception as error:
        import sys
        print(f"Preview failed: {error}", file=sys.stderr)
        raise SystemExit(1)
