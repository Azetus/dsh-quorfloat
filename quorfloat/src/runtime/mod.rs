//! Runtime capabilities that belong to neither the wire nor the panel.
//!
//! Today this is [`diag`]: the two optional files that make a process whose stderr the
//! host swallows observable anyway. The global hotkey joins it in S5 — it is a runtime
//! capability in the same sense, needed by the panel but owned by no view.

pub mod diag;
