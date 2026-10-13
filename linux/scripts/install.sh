#!/bin/sh
# Build and install Kebloop for Linux.
#
#   linux/scripts/install.sh [--prefix DIR] [--uninstall] [--no-build]
#
# PREFIX defaults to ~/.local. DESTDIR is honoured for staged installs. The script never
# calls sudo/doas: for a system-wide prefix run the whole script with the tool of your choice.
set -eu

APP_ID=io.github.kubamus.Kebloop
PREFIX="${PREFIX:-$HOME/.local}"
DESTDIR="${DESTDIR:-}"
action=install
build=1

usage() {
    cat <<USAGE
Usage: $0 [--prefix DIR] [--uninstall] [--no-build]

  --prefix DIR   install under DIR (default: \$HOME/.local)
  --uninstall    remove a previous installation
  --no-build     skip 'cargo build' and use the existing release binary
  DESTDIR        environment variable: stage the files under this root
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || { echo "error: --prefix needs a value" >&2; exit 2; }
            PREFIX=$2
            shift 2
            ;;
        --prefix=*)
            PREFIX=${1#--prefix=}
            shift
            ;;
        --uninstall)
            action=uninstall
            shift
            ;;
        --no-build)
            build=0
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

script_dir=$(cd "$(dirname "$0")" && pwd)
linux_dir=$(dirname "$script_dir")
data_dir=$linux_dir/data

bin_dir=$DESTDIR$PREFIX/bin
share_dir=$DESTDIR$PREFIX/share
icon_dir=$share_dir/icons/hicolor/scalable/apps
apps_dir=$share_dir/applications
meta_dir=$share_dir/metainfo

bin_file=$bin_dir/kebloop
icon_file=$icon_dir/$APP_ID.svg
desktop_file=$apps_dir/$APP_ID.desktop
meta_file=$meta_dir/$APP_ID.metainfo.xml
symbolic_dir=$share_dir/kebloop/icons/hicolor/symbolic/apps

refresh_caches() {
    # Caches are only meaningful for a real (non-staged) install.
    [ -z "$DESTDIR" ] || return 0
    if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ -f "$share_dir/icons/hicolor/index.theme" ]; then
        gtk-update-icon-cache -q -t -f "$share_dir/icons/hicolor" || true
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$apps_dir" || true
    fi
}

if [ "$action" = uninstall ]; then
    rm -f "$bin_file" "$icon_file" "$desktop_file" "$meta_file"
    rm -rf "$share_dir/kebloop/icons"
    rm -rf "$share_dir/kebloop/integrations/gnome-shell"
    rmdir "$share_dir/kebloop/integrations" 2>/dev/null || true
    rmdir "$share_dir/kebloop" 2>/dev/null || true
    refresh_caches
    echo "Removed Kebloop from $DESTDIR$PREFIX"
    echo "Your settings (~/.config/kebloop) and data (~/.local/share/kebloop) were left in place."
    exit 0
fi

if [ "$build" = 1 ]; then
    (cd "$linux_dir" && cargo build --release --locked)
fi

binary=$linux_dir/target/release/kebloop
[ -x "$binary" ] || { echo "error: $binary not found; build first" >&2; exit 1; }

install -d "$bin_dir" "$icon_dir" "$apps_dir" "$meta_dir" "$symbolic_dir"
install -m 0755 "$binary" "$bin_file"
install -m 0644 "$data_dir/$APP_ID.desktop" "$desktop_file"
install -m 0644 "$data_dir/$APP_ID.metainfo.xml" "$meta_file"
# The source icon is a symlink into assets/; copy the real file.
cp -L "$data_dir/icons/hicolor/scalable/apps/$APP_ID.svg" "$icon_file"
chmod 0644 "$icon_file"
# Bundled Adwaita symbolic fallbacks live in Kebloop's own tree, not the system hicolor.
for svg in "$data_dir"/icons/hicolor/symbolic/apps/*.svg; do
    install -m 0644 "$svg" "$symbolic_dir/"
done

# Ship the optional GNOME extension without enabling it in the user's session.
gnome_dir=$share_dir/kebloop/integrations/gnome-shell/kebloop@kebloop.app
install -d "$gnome_dir"
install -m 0644 "$linux_dir/integrations/gnome-shell/kebloop@kebloop.app/extension.js" "$gnome_dir/"
install -m 0644 "$linux_dir/integrations/gnome-shell/kebloop@kebloop.app/metadata.json" "$gnome_dir/"
install -m 0644 "$linux_dir/integrations/gnome-shell/README.md" "$share_dir/kebloop/integrations/gnome-shell/"

refresh_caches

cat <<NEXT
Installed Kebloop to $DESTDIR$PREFIX

Next steps:
  1. Make sure $PREFIX/bin is in your PATH.
  2. Bind a key to "kebloop toggle" in your compositor:
       niri      (config.kdl, binds {})   Alt+Space { spawn "kebloop" "toggle"; }
       Hyprland  (hyprland.conf)          bind = ALT, SPACE, exec, kebloop toggle
       Sway      (config)                 bindsym Mod1+space exec kebloop toggle
       i3        (config)                 bindsym Mod1+space exec --no-startup-id kebloop toggle
     KDE / GNOME: add a custom shortcut that runs "kebloop toggle".
  3. Start it with your session: open Settings > General and enable launch at login
     (this writes an XDG autostart entry running "kebloop --background").
NEXT
