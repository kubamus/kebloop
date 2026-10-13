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


def accessible_content(snapshot):
    if snapshot is None:
        return ""
    names, texts = snapshot
    return "\n".join([*names, *(text for _, text in texts)])


def accessible_candidates(fragment):
    """Describe matching AT-SPI nodes, including their clickable state, for failure output."""
    try:
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
    except Exception:
        return None

    candidates = []
    state_values = (
        ("SHOWING", pyatspi.STATE_SHOWING),
        ("VISIBLE", pyatspi.STATE_VISIBLE),
        ("ENABLED", pyatspi.STATE_ENABLED),
        ("SENSITIVE", pyatspi.STATE_SENSITIVE),
    )

    def visit(node, depth=0, visited=0):
        if depth > 18 or visited >= 2500:
            return
        try:
            name = str(node.name or "")
            if fragment.lower() in name.lower():
                try:
                    state_set = node.getState()
                    states = [label for label, value in state_values if state_set.contains(value)]
                except Exception:
                    states = []
                try:
                    role = str(node.getRoleName())
                except Exception:
                    role = "?"
                try:
                    extents = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                    bounds = (int(extents.x), int(extents.y), int(extents.width), int(extents.height))
                except Exception:
                    bounds = None
                try:
                    actions = node.queryAction()
                    action_names = [actions.getName(index) for index in range(actions.nActions)]
                except Exception:
                    action_names = []
                candidates.append((name, role, states, bounds, action_names))
            count = min(int(node.childCount), 200)
        except Exception:
            return
        next_visited = visited + 1
        for index in range(count):
            try:
                visit(node.getChildAtIndex(index), depth + 1, next_visited)
            except Exception:
                continue

    visit(desktop)
    return candidates


def activate_accessible_button(accessible_name, timeout=8):
    """Activate an exact visible button through its AT-SPI click or activate action."""
    try:
        import pyatspi

        desktop = pyatspi.Registry.getDesktop(0)
    except Exception as error:
        raise AssertionError("AT-SPI is required to activate the note menu button") from error

    state_values = (
        ("SHOWING", pyatspi.STATE_SHOWING),
        ("ENABLED", pyatspi.STATE_ENABLED),
        ("SENSITIVE", pyatspi.STATE_SENSITIVE),
    )
    candidates = []

    def find(node, depth=0, visited=None):
        if visited is None:
            visited = [0]
        if depth > 18 or visited[0] >= 2500:
            return False
        visited[0] += 1
        try:
            name = str(node.name or "")
            if name == accessible_name:
                state_set = node.getState()
                states = [label for label, value in state_values if state_set.contains(value)]
                role = str(node.getRoleName())
                actions = node.queryAction()
                action_names = [actions.getName(index) for index in range(actions.nActions)]
                candidates.append((name, role, states, action_names))
                if (
                    role.casefold() in {"button", "push button"}
                    and all(label in states for label in ("SHOWING", "SENSITIVE"))
                ):
                    for index, action_name in enumerate(action_names):
                        if action_name.casefold() in {"click", "activate"}:
                            try:
                                if actions.doAction(index):
                                    return True
                            except Exception:
                                continue
            count = min(int(node.childCount), 200)
        except Exception:
            try:
                count = min(int(node.childCount), 200)
            except Exception:
                return False
        for index in range(count):
            try:
                result = find(node.getChildAtIndex(index), depth + 1, visited)
                if result:
                    return result
            except Exception:
                continue
        return False

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        candidates.clear()
        if find(desktop):
            return True
        time.sleep(0.2)
    raise AssertionError(
        f"AT-SPI did not expose a sensitive, showing button action for {accessible_name!r}; "
        f"button candidates={candidates[-8:]!r}; "
        f"all candidates={accessible_candidates(accessible_name)!r}"
    )


def window_geometry(window):
    values = {}
    for line in run_xdotool("getwindowgeometry", "--shell", str(window)).splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            values[key] = int(value)
    if "WIDTH" not in values or "HEIGHT" not in values:
        raise AssertionError(f"xdotool did not return dimensions for window {window}: {values!r}")
    return values["WIDTH"], values["HEIGHT"]


def assert_inline_notes(launcher_window):
    wait_until(lambda: bool(visible_windows("^Kebloop$")), "Kebloop launcher window")
    assert launcher_window in visible_windows("^Kebloop$"), "notes left the launcher window"
    assert not visible_windows("^Floating Markdown Notes$"), "notes opened a separate window"


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

        # Open a blank note from root search. Notes stay in the existing launcher window, and the
        # editor is focused without a mouse click.
        type_text("new note")
        time.sleep(0.5)
        press("Return")
        assert_inline_notes(launcher_window)
        wait_until(lambda: run_xdotool("getwindowfocus") == launcher_window, "focus in inline notes")
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
        snapshot = atspi_snapshot()
        if snapshot is not None:
            wait_for_accessible_name("Markdown preview")
            wait_for_accessible_text(
                "Markdown preview",
                lambda text: "BODY_ALPHA" in text and "print(1)" in text,
                "the live preview to render the note while editing",
            )
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

        # Ctrl+Backspace remains text editing while the editor has focus; it must not delete the
        # note. Replace the draft afterward to keep an exact persistence check.
        type_text("EDIT_SENTINEL")
        press("ctrl+BackSpace")
        wait_for_accessible_text(
            "Markdown note editor",
            lambda text: "EDIT_SENTINEL" not in text and "BODY_ALPHA" in text,
            "Ctrl+Backspace to delete text inside the editor",
        )
        assert len(note_files(note_dir)) == 1, "Ctrl+Backspace deleted the note while editing"
        press("ctrl+a")
        type_text(full_source)
        wait_for_accessible_text("Markdown note editor", lambda text: text == full_source, "restored note source")
        press("Escape")
        wait_for_saved(note_dir, full_source)
        assert_inline_notes(launcher_window)

        # Ctrl+K exposes the current note actions.
        press("ctrl+k")
        action_snapshot = wait_for_accessible_name("Copy Markdown")
        if action_snapshot is not None:
            action_content = accessible_content(action_snapshot)
            for label in ("Edit", "Export", "Delete"):
                assert label in action_content, f"Ctrl+K is missing the {label} note action"
        press("Escape")

        # Ctrl+F opens the in-note finder. Escape closes find, then saves to preview.
        press("ctrl+f")
        type_text("BODY_ALPHA")
        press("Escape")
        assert_inline_notes(launcher_window)
        press("Escape")
        assert_inline_notes(launcher_window)

        # Creating from a filtered browser clears its filter and keeps the new blank editor.
        press("ctrl+p")
        type_text("First note")
        wait_for_accessible_text("Markdown preview", lambda text: "BODY_ALPHA" in text, "filtered first note")
        press("ctrl+n")
        assert_inline_notes(launcher_window)
        second_source = "# Second note\n\nBODY_BETA"
        type_text(second_source)
        wait_for_accessible_text("Markdown note editor", lambda text: text == second_source,
                                 "a new note to retain editor focus after clearing the browser filter")
        press("Escape")
        wait_until(lambda: len(note_files(note_dir)) == 2, "second note saved")
        wait_for_saved(note_dir, second_source)

        press("ctrl+p")
        wait_for_accessible_name("Search notes")
        type_text("First note")
        wait_for_accessible_text(
            "Markdown preview",
            lambda text: "BODY_ALPHA" in text,
            "the browser filter to preview its matching note",
        )
        snapshot = atspi_snapshot()
        if snapshot is not None:
            assert "First note" in accessible_content(snapshot), "the filtered note is missing from the browser"

        press("ctrl+a")
        press("BackSpace")
        press("Up")
        wait_for_accessible_text(
            "Markdown preview",
            lambda text: "BODY_BETA" in text,
            "moving through the note list to update its preview",
        )
        press("Return")
        wait_for_accessible_text(
            "Markdown note editor",
            lambda text: text == second_source,
            "Return to edit the selected browser note",
        )
        press("Escape")
        wait_for_saved(note_dir, second_source)

        # Deleting from the browser is immediate and leaves the other note selected.
        press("ctrl+p")
        press("ctrl+BackSpace")
        wait_until(lambda: len(note_files(note_dir)) == 1, "Ctrl+Backspace to delete the browser selection")
        assert all("BODY_BETA" not in text for text in note_texts(note_dir))
        assert_inline_notes(launcher_window)
        if atspi_snapshot() is not None:
            snapshot = atspi_snapshot()
            assert "Delete this note?" not in accessible_content(snapshot), "note deletion showed a confirmation dialog"
        wait_for_accessible_text(
            "Markdown preview",
            lambda text: "BODY_ALPHA" in text,
            "the remaining note to become the preview after deletion",
        )

        # A filter with no matches clears the selected note, so Ctrl+Backspace cannot delete a
        # hidden note. Clearing the query restores the matching row and its preview.
        press("ctrl+p")
        wait_for_accessible_name("Search notes")
        type_text("NO_SUCH_NOTE")
        wait_for_accessible_text(
            "Markdown preview",
            lambda text: bool(text.strip()) and "BODY_ALPHA" not in text,
            "the preview to clear when the note filter has no matches",
        )
        press("ctrl+BackSpace")
        assert len(note_files(note_dir)) == 1, "Ctrl+Backspace deleted a note hidden by the filter"
        assert any("BODY_ALPHA" in text for text in note_texts(note_dir))
        press("ctrl+a")
        press("BackSpace")
        wait_for_accessible_text(
            "Markdown preview",
            lambda text: "BODY_ALPHA" in text,
            "clearing the filter to restore the remaining note preview",
        )

        # Ctrl+K remains available for every note. Activate Delete from the menu to cover its
        # actual action path as well as the keyboard shortcut.
        press("ctrl+k")
        action_snapshot = wait_for_accessible_name("Copy Markdown")
        if action_snapshot is not None:
            action_content = accessible_content(action_snapshot)
            for label in ("Edit", "Export", "Delete"):
                assert label in action_content, f"Ctrl+K is missing the {label} note action"
        activate_accessible_button("Delete")
        try:
            wait_until(
                lambda: len(note_files(note_dir)) == 0,
                "the Delete note menu action",
                timeout=3,
            )
        except AssertionError as error:
            snapshot = atspi_snapshot()
            raise AssertionError(
                f"{error}; Delete candidates={accessible_candidates('Delete')!r}; "
                f"AT-SPI snapshot={None if snapshot is None else snapshot!r}"
            ) from error
        assert_inline_notes(launcher_window)
        if atspi_snapshot() is not None:
            snapshot = atspi_snapshot()
            assert "Delete this note?" not in accessible_content(snapshot), "menu deletion showed a confirmation dialog"

        # Ctrl+Backspace also deletes while the action menu is open.
        menu_source = "# Menu shortcut\n\nMENU_SHORTCUT_MARKER"
        # Wait for GTK to finish releasing the popover's keyboard grab before opening a note.
        wait_until(lambda: run_xdotool("getwindowfocus") == launcher_window, "focus after menu deletion")
        time.sleep(0.25)
        press("ctrl+n")
        time.sleep(0.15)
        type_text(menu_source)
        wait_for_accessible_text("Markdown note editor", lambda text: text == menu_source,
                                 "editor focus after deleting through the note menu")
        press("ctrl+Return")
        wait_for_saved(note_dir, menu_source)
        press("ctrl+k")
        wait_for_accessible_name("Copy Markdown")
        press("ctrl+BackSpace")
        wait_until(lambda: len(note_files(note_dir)) == 0, "Ctrl+Backspace to delete from the note menu")

        # Preview deletion uses the same shortcut and leaves the launcher available.
        press("ctrl+n")
        third_source = "# Third note\n\nBODY_GAMMA"
        type_text(third_source)
        press("Escape")
        wait_for_saved(note_dir, third_source)
        press("ctrl+BackSpace")
        wait_until(lambda: len(note_files(note_dir)) == 0, "Ctrl+Backspace to delete the preview note")
        assert_inline_notes(launcher_window)

        # Fill the browser with enough notes to scroll. The newest note is selected and previewed
        # on entry; scrolling over the list should move the preview to another note automatically.
        scroll_note_count = 12
        scroll_sources = []
        for index in range(1, scroll_note_count + 1):
            source = f"# Scroll {index:02d}\n\nSCROLL_MARKER_{index:02d}"
            scroll_sources.append(source)
            press("ctrl+n")
            type_text(source)
            press("ctrl+Return")
            wait_until(
                lambda count=index: len(note_files(note_dir)) == count,
                f"{index} scroll test notes to be saved",
            )
            wait_for_saved(note_dir, source)

        latest_marker = f"SCROLL_MARKER_{scroll_note_count:02d}"
        press("ctrl+p")
        wait_for_accessible_name("Search notes")
        wait_for_accessible_text(
            "Markdown preview",
            lambda text: latest_marker in text,
            "the newest note to be previewed when opening the browser",
        )
        window_width, window_height = window_geometry(launcher_window)
        assert window_width > 120 and window_height > 100, (
            f"launcher window is too small for the notes browser scroll target: "
            f"{window_width}x{window_height}"
        )
        scroll_x = 100  # inside the 220 px notes list column, away from the preview
        scroll_y = min(max(window_height // 2, 50), window_height - 50)
        run_xdotool("mousemove", "--window", launcher_window, str(scroll_x), str(scroll_y))
        run_xdotool("click", "--repeat", "6", "--delay", "90", "5")
        wait_for_accessible_text(
            "Markdown preview",
            lambda text: "SCROLL_MARKER_" in text and latest_marker not in text,
            "scrolling through the browser to automatically preview another note",
        )

        # Put keyboard focus on the list before clearing it with Ctrl+Backspace.
        run_xdotool(
            "mousemove",
            "--window",
            launcher_window,
            str(scroll_x),
            str(scroll_y),
            "click",
            "1",
        )
        for remaining in range(scroll_note_count, 0, -1):
            press("ctrl+BackSpace")
            wait_until(
                lambda count=remaining - 1: len(note_files(note_dir)) == count,
                f"Ctrl+Backspace to remove scroll note {remaining}",
            )
        assert len(note_files(note_dir)) == 0, "scroll workflow left notes behind"

        # Saving may change the filtered selection. Delete must retain the menu's original target.
        keep_source = "# Keep original\n\nKEEP_ORIGINAL"
        target_source = "# Target original\n\nTARGET_ORIGINAL"
        for source in (keep_source, target_source):
            press("ctrl+n")
            type_text(source)
            press("ctrl+Return")
            wait_for_saved(note_dir, source)
        press("ctrl+p")
        type_text("original")
        wait_for_accessible_text("Markdown preview", lambda text: "TARGET_ORIGINAL" in text,
                                 "the target note in the filtered browser")
        press("Return")
        wait_for_accessible_text("Markdown note editor", lambda text: text == target_source,
                                 "the target note editor")
        press("ctrl+a")
        renamed_source = "# Renamed target\n\nRENAMED_TARGET"
        type_text(renamed_source)
        wait_for_accessible_text("Markdown note editor", lambda text: text == renamed_source,
                                 "the renamed note draft")
        press("ctrl+k")
        wait_for_accessible_name("Copy Markdown")
        press("ctrl+BackSpace")
        wait_until(lambda: len(note_files(note_dir)) == 1, "deletion of the renamed menu target")
        assert note_texts(note_dir) == [keep_source], "Delete removed the note selected after saving instead of its original target"
        press("ctrl+p")
        press("ctrl+BackSpace")
        wait_until(lambda: not note_files(note_dir), "cleanup of the remaining filtered note")

        press("ctrl+w")
        wait_until(lambda: not visible_windows("^Kebloop$"), "Ctrl+W to close the launcher window")

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
        assert_inline_notes(launcher_window)
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
