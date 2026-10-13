#!/usr/bin/env python3
"""Keyboard smoke test for notes and inline utilities.

Run against the built Linux binary in a private Xvfb + D-Bus session with openbox,
python3-gi, and xdotool. AT-SPI is used for text checks when pyatspi is installed. All XDG
data lives in a temporary directory.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

os.environ["GTK_A11Y"] = "atspi"
os.environ["GSK_RENDERER"] = "cairo"

binary = Path(sys.argv[1]).resolve()


def run_xdotool(*arguments):
    return subprocess.run(
        ["xdotool", *arguments],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        timeout=8,
    ).stdout.strip()


def press(*keys):
    run_xdotool("key", "--clearmodifiers", *keys)


def type_text(value):
    lines = value.split("\n")
    for index, line in enumerate(lines):
        if line:
            subprocess.run(
                ["xdotool", "type", "--clearmodifiers", "--delay", "12", line],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                timeout=15,
                text=True,
            )
        if index < len(lines) - 1:
            press("Return")


def wait_until(predicate, description, timeout=10):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.15)
    raise AssertionError(f"timed out waiting for {description}")


def visible_windows(pattern):
    result = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--name", pattern],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        timeout=5,
    )
    return [value for value in result.stdout.splitlines() if value.strip()]


def note_files(note_dir):
    return sorted(note_dir.glob("*.md")) if note_dir.exists() else []


def note_texts(note_dir):
    return [path.read_text(encoding="utf-8") for path in note_files(note_dir)]


def wait_for_saved(note_dir, expected):
    try:
        wait_until(
            lambda: any(text == expected for text in note_texts(note_dir)),
            f"a persisted note matching {expected[:40]!r}",
        )
    except AssertionError as error:
        raise AssertionError(f"{error}; actual saved notes={note_texts(note_dir)!r}") from error


def atspi_snapshot():
    """Return accessible names and text contents, or None if AT-SPI is unavailable."""
    try:
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
    except Exception:
        return None

    names = []
    texts = []
    visited = 0

    def visit(node, depth=0):
        nonlocal visited
        if depth > 18 or visited >= 2500:
            return
        visited += 1
        try:
            name = node.name
            if name:
                names.append(str(name))
            try:
                text = node.queryText()
                value = text.getText(0, text.characterCount)
                if value:
                    texts.append((str(name or ""), str(value)))
            except Exception:
                pass
            count = min(int(node.childCount), 200)
        except Exception:
            return
        for index in range(count):
            try:
                visit(node.getChildAtIndex(index), depth + 1)
            except Exception:
                continue

    visit(desktop)
    return names, texts


def wait_for_accessible_name(fragment, timeout=8):
    snapshot = None
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        snapshot = atspi_snapshot()
        if snapshot is not None and any(fragment in name for name in snapshot[0]):
            return snapshot
        time.sleep(0.2)
    if snapshot is None:
        return None
    raise AssertionError(f"AT-SPI did not expose {fragment!r}; names={snapshot[0]!r}")


def wait_for_accessible_text(accessible_name, predicate, description, timeout=5):
    snapshot = atspi_snapshot()
    if snapshot is None:
        time.sleep(0.4)
        return

    def found():
        current = atspi_snapshot()
        return current is not None and any(
            name == accessible_name and predicate(text)
            for name, text in current[1]
        )

    try:
        wait_until(found, description, timeout)
    except AssertionError as error:
        current = atspi_snapshot()
        raise AssertionError(f"{error}; AT-SPI text={None if current is None else current[1]!r}") from error


with tempfile.TemporaryDirectory(prefix="kebloop-workflows-") as temporary:
    root = Path(temporary)
    env = os.environ.copy()
    env.update(
        XDG_CONFIG_HOME=str(root / "config"),
        XDG_DATA_HOME=str(root / "data"),
        XDG_CACHE_HOME=str(root / "cache"),
        KEBLOOP_APP_ID="app.kebloop.WorkflowSmoke",
        KEBLOOP_DISABLE_SERVICES="shortcuts,tray",
        GTK_A11Y="atspi",
        LC_ALL="C.UTF-8",
        LANGUAGE="en",
    )
    config_dir = Path(env["XDG_CONFIG_HOME"]) / "kebloop"
    config_dir.mkdir(parents=True)
    (config_dir / "config.json").write_text(
        json.dumps(
            {
                "version": 1,
                "activeProfileID": "test",
                "profiles": [{"id": "test", "name": "Test", "settings": {}}],
                "global": {"onboardingDismissed": True},
            }
        ),
        encoding="utf-8",
    )
    note_dir = Path(env["XDG_DATA_HOME"]) / "kebloop" / "notes"
    # Launch the first invocation with `show` so it cannot race an IPC command against
    # GApplication's D-Bus registration. Later `show` calls below exercise the normal IPC path.
    process = subprocess.Popen([binary, "show"], env=env)

    try:
        wait_until(lambda: bool(visible_windows("^Kebloop$")), "launcher window")
        launcher_window = visible_windows("^Kebloop$")[0]

        # Open a blank note from root search. Typing immediately checks that the new editor has
        # focus without a mouse click.
        type_text("new note")
        time.sleep(0.5)
        press("Return")
        wait_until(lambda: bool(visible_windows("^Floating Markdown Notes$")), "floating notes window")
        notes_window = visible_windows("^Floating Markdown Notes$")[0]
        wait_until(
            lambda: run_xdotool("getwindowfocus") == notes_window,
            "keyboard focus in the new note window",
        )
        wait_until(lambda: len(note_files(note_dir)) == 1, "new empty note")

        full_source = (
            "# First note\n\nBODY_ALPHA\n\n```text\nprint(1)\n```\n\n"
            "| Col A | Col B |\n| --- | --- |\n| pear | 3 |\n"
        )
        type_text(full_source)
        wait_for_accessible_text(
            "Markdown note editor",
            lambda text: text == full_source,
            "all Markdown text to reach the focused editor",
        )
        press("Escape")
        wait_for_saved(note_dir, full_source)

        snapshot = atspi_snapshot()
        if snapshot is not None:
            wait_for_accessible_name("Markdown preview")
            snapshot = atspi_snapshot()
            preview_texts = [
                text for name, text in snapshot[1] if name == "Markdown preview"
            ]
            assert preview_texts, "AT-SPI could not read the Markdown preview text"
            preview_text = "\n".join(preview_texts)
            assert "print(1)" in preview_text, "fenced code is missing from the rendered preview"
            assert "Col A" in preview_text and "pear" in preview_text, "table rows are missing from the preview"
            assert "```" not in preview_text, "the preview is exposing raw code-fence markup"
            assert "| Col A |" not in preview_text, "the preview is exposing raw table markup"

        # Return edits; Ctrl+Z must undo a typed change before Escape commits the source.
        press("Return")
        type_text("X")
        wait_for_accessible_text(
            "Markdown note editor",
            lambda text: text == full_source + "X",
            "the undo test text to reach the editor",
        )
        press("ctrl+z")
        wait_for_accessible_text(
            "Markdown note editor",
            lambda text: text == full_source,
            "Ctrl+Z to undo the typed change",
        )
        press("Escape")
        wait_for_saved(note_dir, full_source)
        assert all(text == full_source for text in note_texts(note_dir)), "Ctrl+Z did not undo the edit"

        # Ctrl+F opens the in-note finder. Its first Escape closes find; a second Escape saves to
        # preview. If the first Escape accidentally leaves edit mode, the second closes the note.
        press("ctrl+f")
        type_text("BODY_ALPHA")
        press("Escape")
        wait_until(lambda: bool(visible_windows("^Floating Markdown Notes$")), "find to close without closing notes")
        press("Escape")
        wait_until(lambda: bool(visible_windows("^Floating Markdown Notes$")), "preview after find")

        # Ctrl+P opens the browser. Ctrl+N creates a second note from it; search and Return switch
        # back to the first note without using a pointer.
        press("ctrl+p")
        press("ctrl+n")
        time.sleep(0.3)
        type_text("# Second note\n\nBODY_BETA")
        press("Escape")
        wait_until(lambda: len(note_files(note_dir)) == 2, "second note saved")
        wait_until(lambda: any("BODY_BETA" in text for text in note_texts(note_dir)), "second note contents")
        press("ctrl+p")
        type_text("First note")
        press("Return")
        press("Return")
        type_text("\nSWITCH_MARKER")
        press("Escape")
        wait_until(lambda: any("SWITCH_MARKER" in text and "BODY_ALPHA" in text for text in note_texts(note_dir)), "switch back to first note")
        assert any("BODY_BETA" in text and "SWITCH_MARKER" not in text for text in note_texts(note_dir))

        press("ctrl+w")
        wait_until(lambda: not visible_windows("^Floating Markdown Notes$"), "Ctrl+W to close notes")

        # Dictionary and dictation are reached from the root command list by keyboard. They stay
        # in the launcher; dictation is not started unless Ctrl+Return is pressed.
        subprocess.run([binary, "show"], env=env, check=True, timeout=10)
        wait_until(lambda: bool(visible_windows("^Kebloop$")), "launcher after notes close")
        launcher_window = visible_windows("^Kebloop$")[0]
        type_text("dictionary")
        time.sleep(0.4)
        press("Return")
        dictionary_snapshot = wait_for_accessible_name("Dictionary Lookup")
        assert visible_windows("^Kebloop$")[0] == launcher_window, "dictionary opened outside the launcher"
        press("Escape")
        wait_until(lambda: bool(visible_windows("^Kebloop$")), "dictionary Escape back to root search")

        type_text("dictation")
        time.sleep(0.4)
        press("Return")
        dictation_snapshot = wait_for_accessible_name("Dictation")
        assert visible_windows("^Kebloop$")[0] == launcher_window, "dictation is not inline in the launcher"
        assert not visible_windows("^Floating Markdown Notes$")
        assert not visible_windows("Choose whisper.cpp model"), "dictation unexpectedly opened a model chooser"
        if dictation_snapshot is not None:
            names = dictation_snapshot[0]
            assert any("Choose Model" in name for name in names), "dictation model control is not visible"
            assert any("Record / Stop" in name for name in names), "dictation record control is not visible"
            assert any(name == "Cancel" for name in names), "dictation cancel control is not visible"
        assert not (Path(env["XDG_DATA_HOME"]) / "kebloop" / "dictation-model.txt").exists()
        assert not (Path(env["XDG_CACHE_HOME"]) / "kebloop" / "dictation").exists(), "dictation started recording"

        press("Escape")
        wait_until(lambda: bool(visible_windows("^Kebloop$")), "dictation Escape back to root search")
        # An explicit quit must save an open editor and refuse when recovery storage is unwritable.
        subprocess.run([binary, "command", "newNote"], env=env, check=True, timeout=10)
        wait_until(lambda: bool(visible_windows("^Floating Markdown Notes$")), "note before quitting")
        quit_source = "# Quit note\n\nQUIT_SAVE"
        type_text(quit_source)
        wait_for_accessible_text("Markdown note editor", lambda text: text == quit_source, "quit draft")
        recovery = note_dir / "Recovery"
        recovery.mkdir(exist_ok=True)
        recovery.chmod(0o500)
        try:
            subprocess.run([binary, "quit"], env=env, check=True, timeout=10)
            time.sleep(0.3)
            assert process.poll() is None, "Quit discarded a draft whose save failed"
        finally:
            recovery.chmod(0o700)
        subprocess.run([binary, "quit"], env=env, check=True, timeout=10)
        assert process.wait(timeout=10) == 0
        wait_for_saved(note_dir, quit_source)
        print("GTK keyboard notes, Markdown preview, dictionary and dictation smoke passed")
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=10)
