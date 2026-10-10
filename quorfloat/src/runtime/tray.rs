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

use std::sync::mpsc::Sender;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Runtime};

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

/// Build the menu-bar icon and start delivering its choices to `wake`.
///
/// @param app - the application handle the icon belongs to.
/// @param wake - the window dispatcher's queue; every choice becomes a [`Wake::Tray`].
/// @param marker - where the fact that the icon exists is recorded.
/// @returns the platform's own error if the icon could not be created.
/// @throws tauri::Error - when the icon, the menu or the tray cannot be built.
pub fn install<R: Runtime>(
    app: &AppHandle<R>,
    wake: Sender<Wake>,
    marker: &Marker,
) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, ID_TOGGLE, "显示/隐藏面板", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, ID_RESTART, "重启悬浮窗", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &restart, &quit])?;
    let icon = tauri::image::Image::from_bytes(ICON)?;
    let sender = wake;
    TrayIconBuilder::with_id("quorfloat-tray")
        .icon(icon)
        // Template on macOS: the menu bar's own colour decides, so the icon follows light and
        // dark without two assets. Ignored elsewhere.
        .icon_as_template(true)
        .tooltip("Quorfloat 悬浮窗")
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
    Ok(())
}
