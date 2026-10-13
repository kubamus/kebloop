//! Command-line interface. The same grammar is parsed by the first process (to decide whether to
//! forward quickly to a running instance) and by the primary instance (which executes it).

use std::ffi::OsString;

use clap::{Parser, Subcommand};

/// Application id of the primary instance. `KEBLOOP_APP_ID` overrides it (tests, side-by-side
/// development builds) so a test instance never collides with a real one.
pub const DEFAULT_APP_ID: &str = "io.github.kubamus.Kebloop";

/// The D-Bus application id for this process.
pub fn app_id() -> String {
    std::env::var("KEBLOOP_APP_ID")
        .ok()
        .filter(|id| gtk::gio::Application::id_is_valid(id))
        .unwrap_or_else(|| DEFAULT_APP_ID.to_owned())
}

/// Kebloop: a keyboard-driven launcher.
#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(name = "kebloop", version, about, long_about = None)]
pub struct Cli {
    /// Start without showing the launcher (used by autostart).
    #[arg(long)]
    pub background: bool,
    /// What to do in the running instance.
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

/// Subcommands; each is delivered to the primary instance.
#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum CliCommand {
    /// Show the launcher if hidden, hide it otherwise.
    Toggle,
    /// Show the launcher.
    Show,
    /// Hide the launcher and any other Kebloop panel.
    Hide,
    /// Open the settings window, optionally on a page.
    Settings {
        /// Page to open (for example `general`, `search`, `clipboard`).
        page: Option<String>,
    },
    /// Run a command by id: a built-in (`emoji`, `clipboard`), `snippet:<id>`, `script:<id>`,
    /// `extension:<id>` or `link:<id>`.
    Command {
        /// Command identifier.
        id: String,
    },
    /// Launch an application by desktop-file id (`org.mozilla.firefox.desktop`).
    Launch {
        /// Desktop-file id or absolute path of a `.desktop` file.
        desktop_id: String,
    },
    /// Quit the running instance.
    Quit,
}

/// What the primary instance should do after parsing a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Start (or stay) in the background without touching the UI.
    Background,
    /// Show the launcher.
    Show,
    /// Toggle the launcher.
    Toggle,
    /// Hide all Kebloop panels.
    Hide,
    /// Open settings on a page.
    Settings(Option<String>),
    /// Execute a command id.
    Command(String),
    /// Launch an application.
    Launch(String),
    /// Quit the application.
    Quit,
}

impl Cli {
    /// Parse `args` (including `argv[0]`).
    pub fn parse_from_os<I: IntoIterator<Item = OsString>>(args: I) -> Result<Self, clap::Error> {
        Self::try_parse_from(args)
    }

    /// The request this command line expresses.
    pub fn request(&self) -> Request {
        match &self.command {
            None if self.background => Request::Background,
            None => Request::Show,
            Some(CliCommand::Toggle) => Request::Toggle,
            Some(CliCommand::Show) => Request::Show,
            Some(CliCommand::Hide) => Request::Hide,
            Some(CliCommand::Settings { page }) => Request::Settings(page.clone()),
            Some(CliCommand::Command { id }) => Request::Command(id.clone()),
            Some(CliCommand::Launch { desktop_id }) => Request::Launch(desktop_id.clone()),
            Some(CliCommand::Quit) => Request::Quit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec![OsString::from("kebloop")];
        full.extend(args.iter().map(OsString::from));
        Cli::parse_from_os(full).expect("valid arguments")
    }

    #[test]
    fn bare_invocation_shows_the_launcher() {
        assert_eq!(parse(&[]).request(), Request::Show);
        assert_eq!(parse(&["--background"]).request(), Request::Background);
    }

    #[test]
    fn subcommands_map_to_requests() {
        assert_eq!(parse(&["toggle"]).request(), Request::Toggle);
        assert_eq!(parse(&["show"]).request(), Request::Show);
        assert_eq!(parse(&["hide"]).request(), Request::Hide);
        assert_eq!(parse(&["quit"]).request(), Request::Quit);
        assert_eq!(parse(&["settings"]).request(), Request::Settings(None));
        assert_eq!(
            parse(&["settings", "clipboard"]).request(),
            Request::Settings(Some("clipboard".into()))
        );
        assert_eq!(
            parse(&["command", "snippet:7"]).request(),
            Request::Command("snippet:7".into())
        );
        assert_eq!(
            parse(&["launch", "firefox.desktop"]).request(),
            Request::Launch("firefox.desktop".into())
        );
    }

    #[test]
    fn unknown_subcommands_are_rejected() {
        let result = Cli::parse_from_os(["kebloop", "explode"].map(OsString::from));
        assert!(result.is_err());
    }
}
