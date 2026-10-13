# Bundled icons

`hicolor/scalable/apps/io.github.kubamus.Kebloop.svg` is the Kebloop application icon (MIT).

`hicolor/symbolic/apps/*.svg` are symbolic icons copied unmodified from the GNOME
[Adwaita icon theme](https://gitlab.gnome.org/GNOME/adwaita-icon-theme)
(Copyright the GNOME Project and contributors), used as a fallback for icon themes (for
example Breeze) that lack these names. They are licensed under CC-BY-SA-3.0
(<https://creativecommons.org/licenses/by-sa/3.0/>) or alternatively LGPL-3.0.

They are installed to `<prefix>/share/kebloop/icons` and registered with
`GtkIconTheme::add_search_path`, so the user's own theme always takes precedence.
A test (`icons::tests`) checks that every `*-symbolic` name used in the code is bundled.
