//! Diagnostics: making this process observable from outside.
//!
//! The host captures this process's stderr and surfaces it nowhere, so questions like
//! "did the binary start", "did the approval reach the panel", "what did the host
//! actually send" are unanswerable from outside.
//!
//! The answer is [`append`], with the marker's line format on top:
//!
//! - [`marker`] — short breadcrumbs about what this process decided. Safe to leave on.

pub mod append;
pub mod marker;
