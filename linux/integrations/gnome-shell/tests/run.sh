#!/bin/sh
set -eu
integration_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export integration_dir
task_root=$(mktemp -d /tmp/kebloop-gnome.XXXXXX)
export XDG_CONFIG_HOME="$task_root/config" XDG_DATA_HOME="$task_root/data" XDG_CACHE_HOME="$task_root/cache" XDG_RUNTIME_DIR="$task_root/runtime"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME/gnome-shell/extensions/kebloop@kebloop.app" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
cp "$integration_dir"/kebloop@kebloop.app/* "$XDG_DATA_HOME/gnome-shell/extensions/kebloop@kebloop.app/"
export LIBGL_ALWAYS_SOFTWARE=1 WAYLAND_DISPLAY=kebloop-test
export GSETTINGS_BACKEND=keyfile

exec dbus-run-session sh -c '
set -eu
gsettings set org.gnome.shell enabled-extensions "['"'"'kebloop@kebloop.app'"'"']"
gnome-shell --headless --wayland --no-x11 --wayland-display kebloop-test --virtual-monitor 1280x720 > "$XDG_RUNTIME_DIR/shell.log" 2>&1 &
shell_pid=$!
trap "kill $shell_pid 2>/dev/null || true" EXIT
for attempt in $(seq 1 30); do
    if gdbus call --session --dest org.kebloop.Gnome --object-path /org/kebloop/Gnome --method org.kebloop.Gnome1.Ping > "$XDG_RUNTIME_DIR/ping.log" 2>&1; then break; fi
    sleep 1
done
cat "$XDG_RUNTIME_DIR/ping.log"
python3 "$integration_dir/tests/assert-session.py"
'
