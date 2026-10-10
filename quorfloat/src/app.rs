//! The shell's state: what lives between the wire and the window.
//!
//! This module keeps what the shell owns: the session state machine ([`session`]), the
//! bridge to the frame reader ([`sink`]), the frontend bridge ([`bridge`]: one snapshot
//! shape
//! plus the commands the page can issue), the native-height coordination ([`height`]),
//! and the small local files the process manages — [`preferences`], [`pinned`],
//! [`workspace`], [`geometry`] and [`window_settings`].
//!
//! The drawing itself lives in the web frontend; the Tauri shell in `main.rs` feeds it
//! state and applies its intents to the session.

pub mod bridge;
pub mod height;
pub mod language;
pub mod theme;

pub mod geometry;
pub mod pinned;
pub mod preferences;
pub mod session;
pub mod sink;
pub mod window_settings;
pub mod workspace;
