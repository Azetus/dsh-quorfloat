//! Runtime capabilities that belong to neither the wire nor the panel.
//!
//! [`hotkey`] is the one input the operating system owns, and [`diag`] is the pair of
//! optional files that make a process whose stderr the host swallows observable anyway.
//! Both are capabilities the panel needs and neither belongs to a view.

pub mod diag;
pub mod hotkey;
