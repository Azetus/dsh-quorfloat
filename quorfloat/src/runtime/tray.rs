//! The menu-bar icon: the panel's way of saying "I am running" where no window can say it.
//!
//! The panel is an accessory application, so it has no Dock icon on purpose (that is what makes
//! hiding it hand the keyboard back to whatever the user came from). That leaves the menu bar as
//! the only place this process can be *seen* while Harness is not in front — and, because the
//! icon belongs to this process, its existence is exactly the claim "the sidecar is running".
//! There is no state in which the icon is up and the process is not: a stop takes both away.
//!
//! It also carries the actions that have no other home. The menu is the panel's own, so the
//! choice is delivered to the window dispatcher as a [`Wake`] and handled there, on the one
//! thread that is allowed to touch the window (`main.rs`).
//!
//! **The labels are part of the panel's language.** The four strings a user can read here are
//! the only ones this process draws itself, so they follow `config.window.language` exactly as
//! the frontend's copy follows it. Because the icon belongs to this process and is created once,
//! a language change re-labels the items that already exist ([`TrayMenu::relabel`]) instead of
//! rebuilding the tray: rebuilding would take the icon out of the menu bar and put it back,
//! which a user watching the bar would see as a flicker for a setting that changed nothing
//! about the process.

use std::sync::mpsc::Sender;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Runtime};

use crate::app::language::Language;
use crate::app::sink::Wake;
use crate::runtime::diag::marker::Marker;

/// What the user picked in the menu bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Show the panel when it is hidden, hide it when it is shown.
    ToggleVisibility,
    /// Replace this process: only the host's supervisor can do that, so it is asked.
    Restart,
    /// End this process on purpose, and see that it is not restarted: also the host's business.
    Quit,
}

/// The four strings the menu bar shows, for one language.
///
/// A plain value rather than four accessors so the whole table can be asserted at once, which
/// is how the two languages are kept from drifting apart: every row has to name all four.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayLabels {
    /// The toggle item: show the panel when hidden, hide it when shown.
    pub toggle: &'static str,
    /// The restart item.
    pub restart: &'static str,
    /// The quit item.
    pub quit: &'static str,
    /// The icon's tooltip, shown on hover.
    pub tooltip: &'static str,
}

/// The label table for one language.
///
/// Pure, and the only place these four strings exist: the tray is built from it and a language
/// change re-labels from it, so "what does the menu say in English" has exactly one answer.
///
/// @param language - the language in force.
/// @returns the four strings to draw.
#[must_use]
pub fn labels(language: Language) -> TrayLabels {
    match language {
        Language::Zh => TrayLabels {
            toggle: "显示/隐藏面板",
            restart: "重启悬浮窗",
            quit: "退出",
            tooltip: "Quorfloat 悬浮窗",
        },
        Language::En => TrayLabels {
            toggle: "Show/Hide Panel",
            restart: "Restart Panel",
            quit: "Quit",
            tooltip: "Quorfloat Panel",
        },
    }
}

/// The menu-bar image, embedded so the staged sidecar stays one self-contained executable.
///
/// It is a *template* image: colour is ignored, macOS recolours the alpha channel for light and
/// dark menu bars. The current one is a placeholder (a Phosphor message mark rendered to a
/// monochrome PNG by `scripts/…`); swapping it means re-rendering `icons/tray.svg`.
const ICON: &[u8] = include_bytes!("../../icons/tray.png");

/// Menu item ids, matched back in the menu-event handler below.
const ID_TOGGLE: &str = "quorfloat-toggle";
const ID_RESTART: &str = "quorfloat-restart";
const ID_QUIT: &str = "quorfloat-quit";

/// The live tray: its items and its icon, kept so a language change can re-label them.
///
/// Held rather than discarded because a `TrayIconBuilder` gives no way back to the items it
/// created; without these handles the only way to change a label would be to build a second
/// tray, which is the flicker [`TrayMenu::relabel`] exists to avoid.
///
/// Cloneable on purpose: every field is a cheap handle, so a caller can take a copy and
/// re-label *without holding the lock over the platform call*.
pub struct TrayMenu<R: Runtime> {
    toggle: MenuItem<R>,
    restart: MenuItem<R>,
    quit: MenuItem<R>,
    icon: TrayIcon<R>,
    /// The language the items currently carry, so a no-op change writes no marker.
    language: Language,
}

// Hand-written rather than derived: a derive would require `R: Clone`, and a runtime is not a
// value to copy — Tauri's own handle types implement `Clone` the same way.
impl<R: Runtime> Clone for TrayMenu<R> {
    fn clone(&self) -> Self {
        Self {
            toggle: self.toggle.clone(),
            restart: self.restart.clone(),
            quit: self.quit.clone(),
            icon: self.icon.clone(),
            language: self.language,
        }
    }
}

impl<R: Runtime> TrayMenu<R> {
    /// The language the items are currently labelled in.
    ///
    /// @returns the language the last build or re-label applied.
    #[must_use]
    pub fn language(&self) -> Language {
        self.language
    }

    /// Remember a language without touching the platform.
    ///
    /// Split from [`Self::relabel`] so a caller can claim the change while holding its lock
    /// and then perform the platform calls *after* releasing it: Tauri's `set_text` blocks
    /// until the main thread runs it, and holding a mutex across that is how a dispatcher
    /// thread and a command thread end up waiting on each other.
    ///
    /// @param language - the language to remember.
    pub fn set_language(&mut self, language: Language) {
        self.language = language;
    }

    /// Re-label the existing items and tooltip for a language.
    ///
    /// Every item is attempted even after one fails: a platform that refuses one label is
    /// still better off with the other three in the right language than with the whole menu
    /// in the old one. The language is remembered either way — retrying the same refusal on
    /// every tick would flood the marker file with a failure the user cannot act on.
    ///
    /// @param language - the language to label in.
    /// @param marker - where a per-item refusal is recorded.
    /// @returns whether every item took its new label.
    pub fn relabel(&mut self, language: Language, marker: &Marker) -> bool {
        let table = labels(language);
        let mut complete = true;
        for (item, text) in [
            (&self.toggle, table.toggle),
            (&self.restart, table.restart),
            (&self.quit, table.quit),
        ] {
            if let Err(error) = item.set_text(text) {
                marker.write(&format!("tray item could not be relabelled: {error}"));
                complete = false;
            }
        }
        if let Err(error) = self.icon.set_tooltip(Some(table.tooltip)) {
            marker.write(&format!("tray tooltip could not be relabelled: {error}"));
            complete = false;
        }
        self.language = language;
        complete
    }
}

/// Build the menu-bar icon and start delivering its choices to `wake`.
///
/// @param app - the application handle the icon belongs to.
/// @param wake - the window dispatcher's queue; every choice becomes a [`Wake::Tray`].
/// @param marker - where the fact that the icon exists is recorded.
/// @param language - the panel's language, which decides the four labels.
/// @returns the live tray, so a later language change can re-label it.
/// @throws tauri::Error - when the icon, the menu or the tray cannot be built.
pub fn install<R: Runtime>(
    app: &AppHandle<R>,
    wake: Sender<Wake>,
    marker: &Marker,
    language: Language,
) -> tauri::Result<TrayMenu<R>> {
    let table = labels(language);
    let toggle = MenuItem::with_id(app, ID_TOGGLE, table.toggle, true, None::<&str>)?;
    let restart = MenuItem::with_id(app, ID_RESTART, table.restart, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, table.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &restart, &quit])?;
    let icon = tauri::image::Image::from_bytes(ICON)?;
    let sender = wake;
    let tray = TrayIconBuilder::with_id("quorfloat-tray")
        .icon(icon)
        // Template on macOS: the menu bar's own colour decides, so the icon follows light and
        // dark without two assets. Ignored elsewhere.
        .icon_as_template(true)
        .tooltip(table.tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |_app, event| {
            let action = match event.id().as_ref() {
                ID_TOGGLE => TrayAction::ToggleVisibility,
                ID_RESTART => TrayAction::Restart,
                ID_QUIT => TrayAction::Quit,
                // A future item this build does not know about: nothing to do, and no reason
                // to invent an action for it.
                _ => return,
            };
            // A send failure means the dispatcher is gone, which means the process is ending
            // anyway: there is nothing left to tell.
            let _ = sender.send(Wake::Tray(action));
        })
        .build(app)?;
    marker.write("tray created");
    Ok(TrayMenu { toggle, restart, quit, icon: tray, language })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_labels_follow_the_panels_language() {
        // The four strings are the only ones this process draws itself, so this table is the
        // whole of the tray's localization. It is asserted as a table rather than field by
        // field so that adding a fifth item cannot silently exist in one language only.
        assert_eq!(
            labels(Language::Zh),
            TrayLabels {
                toggle: "显示/隐藏面板",
                restart: "重启悬浮窗",
                quit: "退出",
                tooltip: "Quorfloat 悬浮窗",
            },
        );
        assert_eq!(
            labels(Language::En),
            TrayLabels {
                toggle: "Show/Hide Panel",
                restart: "Restart Panel",
                quit: "Quit",
                tooltip: "Quorfloat Panel",
            },
        );
    }

    #[test]
    fn the_two_languages_do_not_share_a_label() {
        // A row copied from one table to the other is the one mistake a table test cannot
        // see: both languages would render, and the menu would simply be half-translated.
        let zh = labels(Language::Zh);
        let en = labels(Language::En);
        assert_ne!(zh.toggle, en.toggle);
        assert_ne!(zh.restart, en.restart);
        assert_ne!(zh.quit, en.quit);
        assert_ne!(zh.tooltip, en.tooltip);
    }
}
