//! Platform-neutral Kebloop application logic: configuration and profiles, quicklinks, aliases,
//! snippets, shortcut bindings, clipboard history, calculator history, exchange-rate cache,
//! file-search policy, the command registry, window-management and result-action models,
//! script/extension automation and localization. No GUI and no platform integration.
//!
//! Time, paths and the environment are always injected so everything is unit-testable without
//! touching the real home directory.

#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented
    )
)]

pub mod accelerator;
pub mod ai;
pub mod aliases;
pub mod application_control;
pub mod automation;
pub mod calculation_history;
pub mod clipboard;
pub mod commands;
pub mod config;
pub mod currency;
pub mod error;
pub mod file_search;
pub mod l10n;
pub mod persist;
pub mod personalization;
pub mod quicklinks;
pub mod result_action;
pub mod shortcuts;
pub mod snippets;
pub mod util;
pub mod window_management;

pub use error::ConfigError;
