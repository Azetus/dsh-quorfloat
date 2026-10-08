//! The shell's state: what lives between the wire and the window.
//!
//! With the egui view gone (migrated to the Tauri frontend, `frontend/`), this module
//! keeps what the shell still owns: the session state machine ([`session`]), the bridge
//! to the frame reader ([`sink`]), and the small local files the process manages —
//! [`preferences`], [`pinned`], [`workspace`], [`geometry`] and [`window_settings`].
//!
//! The drawing itself lives in the web frontend; the Tauri shell in `main.rs` feeds it
//! state and applies its intents to the session.

mod theme;

pub mod geometry;
pub mod pinned;
pub mod preferences;
pub mod session;
pub mod sink;
pub mod window_settings;
pub mod workspace;
