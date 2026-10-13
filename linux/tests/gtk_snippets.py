#!/usr/bin/env python3
"""Run under a private Xvfb + D-Bus session with openbox, python3-gi and xdotool."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

os.environ["GTK_A11Y"] = "atspi"
os.environ["GSK_RENDERER"] = "cairo"
import gi
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk, GLib

binary = Path(sys.argv[1]).resolve()
with tempfile.TemporaryDirectory(prefix="kebloop-ui-") as temporary:
    env = os.environ.copy()
    env.update(XDG_CONFIG_HOME=temporary + "/config", XDG_DATA_HOME=temporary + "/data",
               XDG_CACHE_HOME=temporary + "/cache", KEBLOOP_APP_ID="app.kebloop.UiSmoke",
               KEBLOOP_DISABLE_SERVICES="shortcuts,tray", GTK_A11Y="atspi")
    config = Path(env["XDG_CONFIG_HOME"]) / "kebloop"
    config.mkdir(parents=True)
    (config / "config.json").write_text(json.dumps({
        "version": 1, "activeProfileID": "test", "profiles": [{
            "id": "test", "name": "Test", "settings": {"snippets": [
                {"id": "greeting", "name": "Greeting", "keyword": ";addr", "template": "Hello {name:Ada}"},
                {"id": "clipboard", "name": "Clipboard", "keyword": ";clip", "template": "{clipboard}"},
            ]}}], "global": {"onboardingDismissed": True, "snippetKeywordExpansionEnabled": True}
    }))
    process = subprocess.Popen([binary, "--background"], env=env)
    app = Gtk.Application(application_id="app.kebloop.EntryFixture")
    failures = []

    def activate(application):
        entry = Gtk.Entry()
        window = Gtk.ApplicationWindow(application=application, title="Kebloop snippet test")
        window.set_child(entry)
        window.present()
        cases = iter([(";addr", "Hello Ada"), (";clip", "Current clipboard")])

        def begin_case():
            try:
                keyword, expected = next(cases)
            except StopIteration:
                application.quit()
                return GLib.SOURCE_REMOVE
            entry.set_text(keyword)
            entry.grab_focus()
            entry.set_position(-1)

            def type_space():
                subprocess.run(["xdotool", "key", "--clearmodifiers", "space"], check=True)
                def verify():
                    if entry.get_text() != expected:
                        failures.append(f"{keyword}: expected {expected!r}, received {entry.get_text()!r}")
                        application.quit()
                    else:
                        begin_case()
                    return GLib.SOURCE_REMOVE
                GLib.timeout_add(1500, verify)
                return GLib.SOURCE_REMOVE
            GLib.timeout_add(500, type_space)
            return GLib.SOURCE_REMOVE

        subprocess.run(["xclip", "-selection", "clipboard"], input=b"Current clipboard",
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        GLib.timeout_add(3000, begin_case)

    app.connect("activate", activate)
    def timed_out():
        failures.append("GTK smoke test timed out")
        app.quit()
        return GLib.SOURCE_REMOVE
    GLib.timeout_add_seconds(20, timed_out)
    try:
        app.run([])
        assert process.poll() is None, "launcher exited unexpectedly"
        if failures:
            raise AssertionError("; ".join(failures))
        for request in [["show"], ["hide"], ["settings", "snippets"], ["hide"], ["quit"]]:
            subprocess.run([binary, *request], env=env, check=True, timeout=10)
        assert process.wait(timeout=10) == 0
        print("GTK snippet expansion, live clipboard rendering and launcher CLI smoke passed")
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
