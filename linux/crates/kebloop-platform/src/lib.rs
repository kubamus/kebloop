//! Linux integration layer for Kebloop (no GTK widgets).
//!
//! Every module takes its environment, paths and policies as parameters so it
//! can be tested without touching the real home directory. Background work
//! runs on dedicated threads and reports through `async-channel`.

pub mod app_control;
pub mod apps;
pub mod autostart;
pub mod clipboard;
pub mod compositor;
pub mod files;
pub mod paste;
pub mod paths;
pub mod power;
pub mod session;
pub mod shortcuts;
pub mod snippet_expansion;
pub mod system_actions;
pub mod text;
pub mod tray;
pub mod window_management;
