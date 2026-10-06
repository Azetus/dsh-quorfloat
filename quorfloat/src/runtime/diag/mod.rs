//! Diagnostics: making this process observable from outside.
//!
//! The host captures this process's stderr and surfaces it nowhere, so questions like
//! "did the binary start", "did the approval reach the panel", "what did the host
//! actually send" are unanswerable from outside — and every one of those has been asked
//! for real during this project.
//!
//! Both answers are the same mechanism ([`append`]) with different line formats:
//!
//! - [`marker`] — short breadcrumbs about what this process decided. Safe to leave on.
//! - [`dump`] — every conversation notification, verbatim, including whatever the
//!   conversation said. A development switch, never set for a user.

pub mod append;
pub mod dump;
pub mod marker;
