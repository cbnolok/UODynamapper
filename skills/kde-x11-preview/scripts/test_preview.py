"""Offline invariants for the preview helper; no live desktop is accessed."""

import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest

import preview


class PreviewTests(unittest.TestCase):
    def test_png_waits_for_completion(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "capture.png"
            self.assertFalse(preview.png_complete(path))
            path.write_bytes(b"")
            self.assertFalse(preview.png_complete(path))
            path.write_bytes(preview.PNG_SIGNATURE + b"unfinished")
            self.assertFalse(preview.png_complete(path))
            path.write_bytes(preview.PNG_SIGNATURE + b"image chunks" + preview.PNG_END)
            self.assertTrue(preview.png_complete(path))
            path.write_bytes(path.read_bytes() + b"trailing incomplete data")
            self.assertFalse(preview.png_complete(path))

    def test_exec_keeps_pid_and_literal_arguments(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "argv.json"
            argument = 'a space; $(touch never-run) `literal` "quoted"'
            program = "import json,os,sys;open(sys.argv[1],'w').write(json.dumps([os.getpid(),sys.argv[2]]))"
            child = subprocess.Popen([
                "/bin/sh", "-c", 'kill -STOP "$$"; exec "$@"', "preview-wrapper",
                sys.executable, "-c", program, str(output), argument,
            ])
            try:
                preview.stopped_child(child, 2)
                self.assertFalse(output.exists())
                child.send_signal(signal.SIGCONT)
                self.assertEqual(child.wait(timeout=3), 0)
                self.assertEqual(json.loads(output.read_text()), [child.pid, argument])
            finally:
                preview.terminate_owned_child(child)

    def test_cleanup_handles_stopped_child_and_leaves_other_child_running(self):
        unrelated = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(15)"])
        owned = subprocess.Popen(["/bin/sh", "-c", 'kill -STOP "$$"; exec sleep 15'])
        try:
            preview.stopped_child(owned, 2)
            preview.terminate_owned_child(owned)
            self.assertIsNotNone(owned.poll())
            self.assertIsNone(unrelated.poll())
        finally:
            preview.terminate_owned_child(owned)
            preview.terminate_owned_child(unrelated)

    @unittest.skipUnless(shutil.which("node"), "Node is needed only to execute the generated JS in this test")
    def test_window_handler_matches_only_pid_and_preserves_desktop_literal(self):
        desktop = 'Workshop PID SERVICE "quoted" \\ path'
        script = preview.kwin_script(12345, desktop, "org.codex.KdePreview.test")
        harness = r"""
const fs = require('fs');
const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const target = {name: input.desktop};
const original = {name: 'User desktop'};
const events = [];
let handler = null;
global.workspace = {desktops: [original, target], windowAdded: {connect(fn) {handler = fn;}}};
global.callDBus = (...args) => events.push(args);
eval(input.script);
const terminal = {pid: 900, desktops: [original], demandsAttention: true};
const preview = {pid: 12345, desktops: [original], demandsAttention: true};
handler(terminal);
handler(preview);
process.stdout.write(JSON.stringify({
 terminal: terminal.desktops.map(d => d.name),
 preview: preview.desktops.map(d => d.name),
 events: events.map(e => e.slice(3)),
}));
"""
        result = subprocess.run(
            ["node", "-e", harness],
            input=json.dumps({"desktop": desktop, "script": script}), text=True,
            capture_output=True, check=True,
        )
        output = json.loads(result.stdout)
        self.assertEqual(output["terminal"], ["User desktop"])
        self.assertEqual(output["preview"], [desktop])
        self.assertEqual(output["events"][0], ["Ready", True])
        self.assertEqual(output["events"][1], ["Placed", "12345", json.dumps([desktop], separators=(",", ":"))])


if __name__ == "__main__":
    unittest.main()
