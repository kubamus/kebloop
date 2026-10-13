//! XDG base directory helpers with an injectable environment.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Directory name used below the XDG base directories.
pub const APP_DIR_NAME: &str = "kebloop";

/// A snapshot of environment variables. Injected so tests never read the real
/// process environment.
#[derive(Debug, Clone, Default)]
pub struct Env {
    vars: HashMap<String, String>,
}

impl Env {
    /// Captures the current process environment (Unicode variables only).
    pub fn from_process() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    /// Builds an environment from explicit pairs (tests).
    pub fn from_pairs<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        Self {
            vars: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    /// Returns a variable; empty values count as unset (as the XDG spec requires).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars
            .get(name)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// Returns a variable only when it is an absolute path (XDG rule).
    pub fn get_abs_path(&self, name: &str) -> Option<PathBuf> {
        self.get(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    }

    /// Sets a variable (builder style, tests).
    #[must_use]
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_owned(), value.to_owned());
        self
    }

    /// Directories listed in `PATH`.
    pub fn path_dirs(&self) -> Vec<PathBuf> {
        self.get("PATH")
            .map(|p| {
                p.split(':')
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Resolved XDG base directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdgDirs {
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub data_home: PathBuf,
    pub cache_home: PathBuf,
    pub state_home: PathBuf,
    pub runtime_dir: Option<PathBuf>,
    pub config_dirs: Vec<PathBuf>,
    pub data_dirs: Vec<PathBuf>,
}

impl XdgDirs {
    /// Resolves the directories from the process environment.
    pub fn from_process_env() -> Self {
        Self::from_env(&Env::from_process())
    }

    /// Resolves the directories from `env`, applying the XDG defaults.
    pub fn from_env(env: &Env) -> Self {
        let home = env
            .get_abs_path("HOME")
            .unwrap_or_else(|| PathBuf::from("/"));
        let under_home =
            |name: &str, rel: &str| env.get_abs_path(name).unwrap_or_else(|| home.join(rel));
        let abs_list = |name: &str, default: &[&str]| -> Vec<PathBuf> {
            let list: Vec<PathBuf> = env
                .get(name)
                .map(|v| {
                    v.split(':')
                        .map(PathBuf::from)
                        .filter(|p| p.is_absolute())
                        .collect()
                })
                .unwrap_or_default();
            if list.is_empty() {
                default.iter().map(PathBuf::from).collect()
            } else {
                list
            }
        };
        Self {
            config_home: under_home("XDG_CONFIG_HOME", ".config"),
            data_home: under_home("XDG_DATA_HOME", ".local/share"),
            cache_home: under_home("XDG_CACHE_HOME", ".cache"),
            state_home: under_home("XDG_STATE_HOME", ".local/state"),
            runtime_dir: env.get_abs_path("XDG_RUNTIME_DIR"),
            config_dirs: abs_list("XDG_CONFIG_DIRS", &["/etc/xdg"]),
            data_dirs: abs_list("XDG_DATA_DIRS", &["/usr/local/share", "/usr/share"]),
            home,
        }
    }

    /// `$XDG_CONFIG_HOME/kebloop`
    pub fn app_config_dir(&self) -> PathBuf {
        self.config_home.join(APP_DIR_NAME)
    }

    /// `$XDG_DATA_HOME/kebloop`
    pub fn app_data_dir(&self) -> PathBuf {
        self.data_home.join(APP_DIR_NAME)
    }

    /// `$XDG_CACHE_HOME/kebloop`
    pub fn app_cache_dir(&self) -> PathBuf {
        self.cache_home.join(APP_DIR_NAME)
    }

    /// `$XDG_RUNTIME_DIR/kebloop` (falls back to the cache dir when there is no runtime dir).
    pub fn app_runtime_dir(&self) -> PathBuf {
        self.runtime_dir
            .as_deref()
            .map_or_else(|| self.app_cache_dir(), |d| d.join(APP_DIR_NAME))
    }

    /// `$XDG_CACHE_HOME/kebloop/application-index.json`
    pub fn application_index_cache(&self) -> PathBuf {
        self.app_cache_dir().join("application-index.json")
    }

    /// `$XDG_CONFIG_HOME/user-dirs.dirs`
    pub fn user_dirs_file(&self) -> PathBuf {
        self.config_home.join("user-dirs.dirs")
    }

    /// `$XDG_CONFIG_HOME/autostart`
    pub fn autostart_dir(&self) -> PathBuf {
        self.config_home.join("autostart")
    }
}

/// Creates `dir` (and parents) with mode 0700 on Unix.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_home() {
        let env = Env::from_pairs([("HOME", "/home/u")]);
        let x = XdgDirs::from_env(&env);
        assert_eq!(x.config_home, Path::new("/home/u/.config"));
        assert_eq!(x.data_home, Path::new("/home/u/.local/share"));
        assert_eq!(x.cache_home, Path::new("/home/u/.cache"));
        assert_eq!(
            x.data_dirs,
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share")
            ]
        );
        assert_eq!(x.config_dirs, vec![PathBuf::from("/etc/xdg")]);
        assert_eq!(x.runtime_dir, None);
        assert_eq!(
            x.application_index_cache(),
            Path::new("/home/u/.cache/kebloop/application-index.json")
        );
    }

    #[test]
    fn env_overrides_and_ignores_relative_or_empty() {
        let env = Env::from_pairs([
            ("HOME", "/home/u"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("XDG_DATA_HOME", "relative/data"),
            ("XDG_CACHE_HOME", ""),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_DATA_DIRS", "/a:rel:/b::/c"),
        ]);
        let x = XdgDirs::from_env(&env);
        assert_eq!(x.config_home, Path::new("/cfg"));
        assert_eq!(x.data_home, Path::new("/home/u/.local/share"));
        assert_eq!(x.cache_home, Path::new("/home/u/.cache"));
        assert_eq!(x.app_runtime_dir(), Path::new("/run/user/1000/kebloop"));
        assert_eq!(
            x.data_dirs,
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c")
            ]
        );
    }

    #[test]
    fn runtime_dir_falls_back_to_cache() {
        let x = XdgDirs::from_env(&Env::from_pairs([("HOME", "/h")]));
        assert_eq!(x.app_runtime_dir(), Path::new("/h/.cache/kebloop"));
    }

    #[test]
    fn path_dirs_split() {
        let env = Env::from_pairs([("PATH", "/usr/bin::/bin")]);
        assert_eq!(
            env.path_dirs(),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")]
        );
    }
}
