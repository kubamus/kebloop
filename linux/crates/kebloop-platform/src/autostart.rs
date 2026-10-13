//! XDG autostart entry (`$XDG_CONFIG_HOME/autostart/io.github.kubamus.Kebloop.desktop`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::paths::XdgDirs;

pub const APP_ID: &str = "io.github.kubamus.Kebloop";
pub const DEFAULT_EXEC: &str = "kebloop --background";

/// Directories whose binaries are found through the default `PATH` of every session.
const SYSTEM_BIN_DIRS: [&str; 4] = ["/usr/bin", "/usr/local/bin", "/bin", "/usr/sbin"];

/// Quote one argument of a desktop-entry `Exec` key: wrapped in double quotes when it contains
/// reserved characters, with `"`, `` ` ``, `$` and `\` escaped, and each backslash doubled
/// again for the string-level escaping of the key. `%` becomes `%%` (field codes).
pub fn exec_quote(arg: &str) -> String {
    let reserved = |c: char| " \t\n\"'\\><~|&;$*?#()`".contains(c);
    let arg = arg.replace('%', "%%");
    if !arg.is_empty() && !arg.chars().any(reserved) {
        return arg;
    }
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        match c {
            '\\' => out.push_str("\\\\\\\\"),
            '"' | '`' | '$' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `Exec` line for starting this installation in the background. The absolute path of the
/// running executable is used unless the binary lives in a standard system prefix (where the
/// bare name resolves in every session); without a known executable the bare name is used.
pub fn exec_line(current_exe: Option<&Path>, args: &[&str]) -> String {
    let program = match current_exe {
        Some(exe)
            if exe.is_absolute()
                && !exe
                    .parent()
                    .is_some_and(|dir| SYSTEM_BIN_DIRS.iter().any(|d| dir == Path::new(d))) =>
        {
            exec_quote(&exe.to_string_lossy())
        }
        _ => "kebloop".to_owned(),
    };
    let mut line = program;
    for arg in args {
        line.push(' ');
        line.push_str(&exec_quote(arg));
    }
    line
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autostart {
    path: PathBuf,
    exec: String,
}

impl Autostart {
    /// Entry at an explicit path (tests) with the default `Exec`.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            exec: DEFAULT_EXEC.to_owned(),
        }
    }

    /// Entry in `$XDG_CONFIG_HOME/autostart`.
    pub fn for_dirs(xdg: &XdgDirs) -> Self {
        Self::at(xdg.autostart_dir().join(format!("{APP_ID}.desktop")))
    }

    /// Overrides the `Exec` line (for example an absolute path to the binary).
    #[must_use]
    pub fn with_exec(mut self, exec: impl Into<String>) -> Self {
        self.exec = exec.into();
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Desktop-entry content written by [`enable`](Self::enable).
    pub fn contents(&self) -> String {
        format!(
            "[Desktop Entry]\nType=Application\nName=Kebloop\nComment=Keyboard-driven launcher\n\
             Exec={}\nIcon={APP_ID}\nTerminal=false\nStartupNotify=false\n\
             X-GNOME-Autostart-enabled=true\nX-KDE-autostart-after=panel\n",
            self.exec
        )
    }

    /// Writes the entry atomically.
    pub fn enable(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("desktop.tmp");
        fs::write(&tmp, self.contents())?;
        fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }

    /// Removes the entry; a missing file is not an error.
    pub fn disable(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// True when the entry exists and is not switched off by `Hidden=true` or
    /// `X-GNOME-Autostart-enabled=false`.
    pub fn is_enabled(&self) -> bool {
        let Ok(content) = fs::read_to_string(&self.path) else {
            return false;
        };
        !content.lines().map(str::trim).any(|l| {
            let l = l.replace(' ', "");
            l.eq_ignore_ascii_case("Hidden=true")
                || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Env;
    use tempfile::TempDir;

    #[test]
    fn enable_disable_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let env = Env::from_pairs([
            ("HOME", tmp.path().to_str().unwrap()),
            ("XDG_CONFIG_HOME", tmp.path().join("cfg").to_str().unwrap()),
        ]);
        let auto = Autostart::for_dirs(&XdgDirs::from_env(&env));
        assert_eq!(
            auto.path(),
            tmp.path()
                .join("cfg/autostart/io.github.kubamus.Kebloop.desktop")
        );
        assert!(!auto.is_enabled());
        auto.enable().unwrap();
        assert!(auto.is_enabled());
        let content = fs::read_to_string(auto.path()).unwrap();
        assert!(content.contains("Exec=kebloop --background\n"));
        assert!(content.starts_with("[Desktop Entry]\n"));
        auto.enable().unwrap();
        auto.disable().unwrap();
        assert!(!auto.is_enabled());
        auto.disable().unwrap();
    }

    #[test]
    fn exec_quoting_follows_the_desktop_entry_spec() {
        assert_eq!(exec_quote("/usr/bin/kebloop"), "/usr/bin/kebloop");
        assert_eq!(exec_quote("--background"), "--background");
        assert_eq!(
            exec_quote("/opt/my apps/kebloop"),
            "\"/opt/my apps/kebloop\""
        );
        assert_eq!(exec_quote("/o/a\"b"), "\"/o/a\\\"b\"");
        assert_eq!(exec_quote("/o/$x`y"), "\"/o/\\$x\\`y\"");
        assert_eq!(exec_quote("/o/a\\b"), "\"/o/a\\\\\\\\b\"");
        assert_eq!(exec_quote("100%"), "100%%");
        assert_eq!(exec_quote(""), "\"\"");
    }

    #[test]
    fn exec_line_prefers_absolute_path_outside_system_prefixes() {
        let args = ["--background"];
        assert_eq!(
            exec_line(Some(Path::new("/usr/bin/kebloop")), &args),
            "kebloop --background"
        );
        assert_eq!(
            exec_line(Some(Path::new("/usr/local/bin/kebloop")), &args),
            "kebloop --background"
        );
        assert_eq!(
            exec_line(Some(Path::new("/home/u/.cargo/bin/kebloop")), &args),
            "/home/u/.cargo/bin/kebloop --background"
        );
        assert_eq!(
            exec_line(Some(Path::new("/home/u/My Apps/kebloop")), &args),
            "\"/home/u/My Apps/kebloop\" --background"
        );
        assert_eq!(exec_line(None, &args), "kebloop --background");
    }

    #[test]
    fn disabled_flags_are_respected() {
        let tmp = TempDir::new().unwrap();
        let auto =
            Autostart::at(tmp.path().join("a.desktop")).with_exec("/opt/kebloop --background");
        auto.enable().unwrap();
        assert!(
            fs::read_to_string(auto.path())
                .unwrap()
                .contains("Exec=/opt/kebloop --background")
        );
        fs::write(auto.path(), "[Desktop Entry]\nHidden=true\n").unwrap();
        assert!(!auto.is_enabled());
        fs::write(
            auto.path(),
            "[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n",
        )
        .unwrap();
        assert!(!auto.is_enabled());
    }
}
