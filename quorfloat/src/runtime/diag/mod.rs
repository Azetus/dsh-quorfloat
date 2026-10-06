//! Diagnostics: making this process observable from outside.
//!
//! The host captures this process's stderr and surfaces it nowhere, so questions like
//! "did the binary start", "did the approval reach the panel", "what did the host
//! actually send" are unanswerable from outside — and every one of those has been asked
//! for real during this project.
//!
//! The answer is [`append`], with the marker's line format on top:
//!
//! - [`marker`] — short breadcrumbs about what this process decided. Safe to leave on.
//!   (The raw-frame `dump` left with the egui view: there is no frame to dump anymore,
//!   and the decision to abandon it is recorded in `docs/tauri-migration-plan.md` §1.)

pub mod append;
pub mod marker;
