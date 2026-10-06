//! Colours, sizes, and the one string the panel asks the host for.
//!
//! Gathered so that a change of palette is one file rather than a search, and so the
//! drawing code reads as layout instead of as hexadecimal.

use eframe::egui;

/// How tall the conversation is allowed to be when nothing else needs the space.
///
/// A floor, not a preference: the panel's reason to exist is the conversation, so it
/// gets its own room even when the window is small.
pub(super) const CONVERSATION_MIN_HEIGHT: f32 = 160.0;

/// The locale the panel asks the asker's own text for.
///
/// Every string this file writes is Chinese, so a localized `displayReason` is
/// requested in Chinese too and falls back to `en` (see [`Interaction::detail`]).
pub(crate) const DISPLAY_LOCALE: &str = "zh";

pub(super) const BACKGROUND: egui::Color32 = egui::Color32::from_rgb(24, 24, 28);

pub(super) const CARD: egui::Color32 = egui::Color32::from_rgb(34, 34, 41);

pub(super) const TEXT: egui::Color32 = egui::Color32::from_rgb(226, 226, 234);

pub(super) const MUTED: egui::Color32 = egui::Color32::from_rgb(150, 150, 165);

pub(super) const ALLOW: egui::Color32 = egui::Color32::from_rgb(38, 92, 58);

pub(super) const REJECT: egui::Color32 = egui::Color32::from_rgb(104, 42, 46);

pub(super) const BANNER: egui::Color32 = egui::Color32::from_rgb(58, 50, 30);

pub(super) const WARN: egui::Color32 = egui::Color32::from_rgb(228, 196, 122);

pub(super) const OK: egui::Color32 = egui::Color32::from_rgb(134, 205, 158);

pub(super) const BAD: egui::Color32 = egui::Color32::from_rgb(226, 140, 140);
