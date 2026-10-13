//! Icon helpers: applying a [`IconSpec`] to a `GtkImage`, the app icon and the debug-build
//! icon search path.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gtk::gio;

use crate::launcher::model::IconSpec;

/// Icon name of the application (installed as `hicolor/scalable/apps/<id>.svg`).
pub const APP_ICON_NAME: &str = "io.github.kubamus.Kebloop";

const FALLBACK_APP_ICON: &str = "application-x-executable";

/// Candidate directories holding Kebloop's bundled `hicolor/symbolic/apps` fallback icons.
///
/// Debug builds use the repository tree; release builds use `<prefix>/share/kebloop/icons`
/// (derived from the executable) and every XDG data dir.
fn bundled_icon_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(debug_assertions) {
        dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/icons"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(Path::parent)
    {
        dirs.push(prefix.join("share/kebloop/icons"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let data_dirs = if data_dirs.is_empty() {
        "/usr/local/share:/usr/share".to_owned()
    } else {
        data_dirs
    };
    let home_data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")));
    for base in home_data
        .into_iter()
        .chain(data_dirs.split(':').map(PathBuf::from))
    {
        dirs.push(base.join("kebloop/icons"));
    }
    dirs.retain(|d| d.is_dir());
    dirs.dedup();
    dirs
}

/// Make the bundled symbolic icons visible to the icon theme. GTK treats search-path entries
/// as theme roots, so `hicolor/symbolic/apps` inside them is the last-resort fallback: the
/// user's icon theme still wins whenever it provides the icon.
pub fn register_development_icons() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);
    for dir in bundled_icon_dirs() {
        theme.add_search_path(&dir);
    }
}

thread_local! {
    static CONTENT_ICONS: RefCell<HashMap<String, gio::Icon>> = RefCell::new(HashMap::new());
}

fn content_type_icon(file_name: &str, is_dir: bool) -> gio::Icon {
    let content_type = if is_dir {
        "inode/directory".to_owned()
    } else {
        gio::content_type_guess(Some(file_name), None).0.to_string()
    };
    CONTENT_ICONS.with(|cache| {
        cache
            .borrow_mut()
            .entry(content_type.clone())
            .or_insert_with(|| gio::content_type_get_icon(&content_type))
            .clone()
    })
}

fn has_icon(name: &str) -> bool {
    gtk::gdk::Display::default()
        .map(|display| gtk::IconTheme::for_display(&display).has_icon(name))
        .unwrap_or(false)
}

/// Show `spec` in `image`. Clipboard thumbnails are the caller's business (it sets a paintable).
pub fn apply(image: &gtk::Image, spec: &IconSpec, pixel_size: i32) {
    image.set_pixel_size(pixel_size);
    match spec {
        IconSpec::Symbolic(name) => {
            if has_icon(name) {
                image.set_icon_name(Some(name));
            } else {
                image.set_icon_name(Some("application-x-executable-symbolic"));
            }
        }
        IconSpec::App(icon) => match icon.as_deref() {
            Some(path) if path.starts_with('/') => {
                let file = gio::File::for_path(path);
                image.set_from_gicon(&gio::FileIcon::new(&file));
            }
            Some(name) if has_icon(name) => image.set_icon_name(Some(name)),
            _ => image.set_icon_name(Some(FALLBACK_APP_ICON)),
        },
        IconSpec::File { path, is_dir } => {
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            image.set_from_gicon(&content_type_icon(&name, *is_dir));
        }
        IconSpec::ClipboardImage(_) => image.set_icon_name(Some("image-x-generic-symbolic")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Names provided by libadwaita's own resources.
    const LIBADWAITA_PROVIDED: &[&str] = &["adw-external-link-symbolic"];

    fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rs_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn every_symbolic_icon_literal_is_bundled() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../");
        let icons = root.join("data/icons/hicolor/symbolic/apps");
        let mut files = Vec::new();
        rs_files(&root.join("crates"), &mut files);
        let mut names = BTreeSet::new();
        for file in files {
            let text = fs::read_to_string(&file).unwrap();
            for part in text.split('"').skip(1).step_by(2) {
                if part.ends_with("-symbolic")
                    && part
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    names.insert(part.to_owned());
                }
            }
        }
        assert!(names.len() > 10, "scan found too few icon names");
        let missing: Vec<_> = names
            .iter()
            .filter(|n| !LIBADWAITA_PROVIDED.contains(&n.as_str()))
            .filter(|n| !icons.join(format!("{n}.svg")).is_file())
            .collect();
        assert!(
            missing.is_empty(),
            "symbolic icons not bundled in data/icons: {missing:?}"
        );
    }
}
