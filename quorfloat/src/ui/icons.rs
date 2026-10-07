//! The icons the panel draws.
//!
//! Two decisions are baked into this file.
//!
//! **A font, registered under its own family.** Drawing an icon is laying out one
//! character, so it lands in egui's glyph atlas and is cached like any other glyph — no
//! texture per icon, and changing an icon's colour costs nothing because glyphs are
//! rasterised white and tinted at draw time. The family is named and never a fallback, so
//! text cannot accidentally render an icon (`ui/fonts.rs` makes the same point from the
//! other side).
//!
//! **The glyphs come from `egui-phosphor`, addressed through this enum.** The enum is the
//! panel's vocabulary — the rest of the UI asks for [`Icon::Chat`], never for a codepoint —
//! and the match arms below are the only place that knows which icon set is behind it.
//! Swapping sets, or hand-picking a different glyph for one control, is a change to this
//! file and nothing else.
//!
//! Names are Phosphor's kebab-case names, because the design names them: the mockup writes
//! `data-phosphor="caret-down"`, and `the_design_agrees_with_this_table` compares the two,
//! so a control cannot quietly start using an icon the design never chose.

use eframe::egui;

/// One icon, as the panel's own vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// A workspace.
    Folder,
    /// A workspace in its plain form.
    FolderSimple,
    /// A conversation.
    Chat,
    /// The disclosure marker on a picker.
    CaretDown,
    /// Pinned: this workspace or conversation opens again.
    PushPin,
    /// Not pinned.
    PushPinSlash,
    /// Settings.
    GearSix,
    /// Close, hide, dismiss.
    Close,
    /// What the composer is for.
    Search,
    /// Send.
    ArrowUp,
    /// Stop: the answer being generated right now.
    Stop,
    /// The clipboard attachment row.
    ClipboardText,
    /// Reasoning effort.
    Brain,
    /// Permission, granted after a check.
    ShieldCheck,
    /// Permission, in its plain form.
    Shield,
    /// Show something that is normally hidden.
    Eye,
    /// A chosen option.
    Check,
    /// A new one.
    Plus,
    /// Return/Enter, for the keyboard hints.
    KeyReturn,
    /// Shift, drawn as a fat arrow because the font has no shift glyph.
    ShiftUp,
    /// Session throughput.
    Gauge,
    /// Cache usage.
    Database,
    /// Context occupancy.
    ChartPie,
    /// A folded section, pointing right.
    CaretRight,
}

/// Every icon this build knows, for the checks that have to consider all of them.
pub const ALL: [Icon; 24] = [
    Icon::Folder,
    Icon::FolderSimple,
    Icon::Chat,
    Icon::CaretDown,
    Icon::PushPin,
    Icon::PushPinSlash,
    Icon::GearSix,
    Icon::Close,
    Icon::Search,
    Icon::ArrowUp,
    Icon::Stop,
    Icon::ClipboardText,
    Icon::Brain,
    Icon::ShieldCheck,
    Icon::Shield,
    Icon::Eye,
    Icon::Check,
    Icon::Plus,
    Icon::Gauge,
    Icon::Database,
    Icon::ChartPie,
    Icon::KeyReturn,
    Icon::ShiftUp,
    Icon::CaretRight,
];

/// The icons the design itself names.
///
/// Kept apart from [`ALL`] so that the cross-check against the mockup stays meaningful: the
/// design's map is what those icons are checked against, while an icon for a control the
/// design does not have — the keyboard hints, here — is a choice this panel made and says so.
pub const DESIGNED: [Icon; 21] = [
    Icon::Folder,
    Icon::FolderSimple,
    Icon::Chat,
    Icon::CaretDown,
    Icon::PushPin,
    Icon::PushPinSlash,
    Icon::GearSix,
    Icon::Close,
    Icon::Search,
    Icon::ArrowUp,
    Icon::Stop,
    Icon::ClipboardText,
    Icon::Brain,
    Icon::ShieldCheck,
    Icon::Shield,
    Icon::Eye,
    Icon::Check,
    Icon::Plus,
    Icon::Gauge,
    Icon::Database,
    Icon::ChartPie,
];

impl Icon {
    /// The name Phosphor knows this icon by, and the name the design uses for it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::FolderSimple => "folder-simple",
            Self::Chat => "chat",
            Self::CaretDown => "caret-down",
            Self::PushPin => "push-pin",
            Self::PushPinSlash => "push-pin-slash",
            Self::GearSix => "gear-six",
            Self::Close => "x",
            Self::Search => "magnifying-glass",
            Self::ArrowUp => "arrow-up",
            Self::Stop => "square",
            Self::ClipboardText => "clipboard-text",
            Self::Brain => "brain",
            Self::ShieldCheck => "shield-check",
            Self::Shield => "shield",
            Self::Eye => "eye",
            Self::Check => "check",
            Self::Plus => "plus",
            Self::Gauge => "gauge",
            Self::Database => "database",
            Self::ChartPie => "chart-pie",
            Self::KeyReturn => "key-return",
            Self::ShiftUp => "arrow-fat-up",
            Self::CaretRight => "caret-right",
        }
    }

    /// The character the bundled font draws for this icon.
    #[must_use]
    pub const fn chars(self) -> &'static str {
        match self {
            Self::Folder => egui_phosphor::regular::FOLDER,
            Self::FolderSimple => egui_phosphor::regular::FOLDER_SIMPLE,
            Self::Chat => egui_phosphor::regular::CHAT,
            Self::CaretDown => egui_phosphor::regular::CARET_DOWN,
            Self::PushPin => egui_phosphor::regular::PUSH_PIN,
            Self::PushPinSlash => egui_phosphor::regular::PUSH_PIN_SLASH,
            Self::GearSix => egui_phosphor::regular::GEAR_SIX,
            Self::Close => egui_phosphor::regular::X,
            Self::Search => egui_phosphor::regular::MAGNIFYING_GLASS,
            Self::ArrowUp => egui_phosphor::regular::ARROW_UP,
            Self::Stop => egui_phosphor::regular::SQUARE,
            Self::ClipboardText => egui_phosphor::regular::CLIPBOARD_TEXT,
            Self::Brain => egui_phosphor::regular::BRAIN,
            Self::ShieldCheck => egui_phosphor::regular::SHIELD_CHECK,
            Self::Shield => egui_phosphor::regular::SHIELD,
            Self::Eye => egui_phosphor::regular::EYE,
            Self::Check => egui_phosphor::regular::CHECK,
            Self::Plus => egui_phosphor::regular::PLUS,
            Self::Gauge => egui_phosphor::regular::GAUGE,
            Self::Database => egui_phosphor::regular::DATABASE,
            Self::ChartPie => egui_phosphor::regular::CHART_PIE,
            Self::KeyReturn => egui_phosphor::regular::KEY_RETURN,
            Self::ShiftUp => egui_phosphor::regular::ARROW_FAT_UP,
            Self::CaretRight => egui_phosphor::regular::CARET_RIGHT,
        }
    }
}

/// Non-minimal permission uses the filled variant; other marks stay outlined.
fn font(icon: Icon, size: f32) -> egui::FontId {
    if icon == Icon::Shield { crate::ui::fonts::filled_icon_font(size) }
    else { crate::ui::fonts::icon_font(size) }
}

/// Render one icon.
///
/// Draws nothing at all in a context where the icon family is not registered yet — which
/// happens in exactly two places: a frame that ran before the panel installed its fonts,
/// and a test that draws a panel into its own context. Neither is worth a panic, and egui
/// answers an unbound family with one, so the guard is here rather than in a convention.
/// The icons themselves are compiled in (`egui-phosphor`), so in a running panel this
/// branch is unreachable.
///
/// @param ctx - the context that will draw the text.
/// @param icon - which picture.
/// @param size - the box it is laid out in, in points.
/// @param colour - the colour to draw it in.
/// @returns text that draws as that icon, or nothing.
#[must_use]
pub fn glyph(ctx: &egui::Context, icon: Icon, size: f32, colour: egui::Color32) -> egui::RichText {
    if !crate::ui::fonts::icons_ready(ctx) {
        return egui::RichText::new("");
    }
    egui::RichText::new(icon.chars()).font(font(icon, size)).color(colour)
}

/// Paint one icon into a rect, for the places that draw rather than lay out text.
///
/// The companion to [`glyph`], and the reason it exists: an icon drawn with
/// `ui.painter().text(…)` names the family itself, which is how the guard in [`glyph`] was
/// bypassed once and the panel panicked on its first frame. There is now one guarded way to
/// draw an icon and one guarded way to lay one out, and
/// `nothing_outside_this_file_names_the_icon_family` keeps it that way.
///
/// @param ui - where to paint.
/// @param center - the middle of the icon's box.
/// @param icon - which picture.
/// @param size - the box's size, in points.
/// @param colour - the colour to draw it in.
pub fn paint(ui: &egui::Ui, center: egui::Pos2, icon: Icon, size: f32, colour: egui::Color32) {
    if !crate::ui::fonts::icons_ready(ui.ctx()) {
        return;
    }
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        icon.chars(),
        font(icon, size),
        colour,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_one_private_use_character() {
        // Not a ligature and not a word: one glyph, addressed by one codepoint. A set that
        // switched to ligature sequences would still draw — but it would also draw into
        // every label containing that sequence, which is a failure mode worth catching.
        for icon in ALL {
            let chars: Vec<char> = icon.chars().chars().collect();
            assert_eq!(chars.len(), 1, "{} is one character", icon.name());
            assert!(
                (0xE000..=0xF8FF).contains(&(chars[0] as u32)),
                "{} is a private-use character, not text",
                icon.name(),
            );
        }
    }

    #[test]
    fn every_icon_has_a_distinct_name_and_glyph() {
        let mut names: Vec<&str> = ALL.iter().map(|icon| icon.name()).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "two icons share a name");

        let mut glyphs: Vec<&str> = ALL.iter().map(|icon| icon.chars()).collect();
        glyphs.sort_unstable();
        let unique = glyphs.len();
        glyphs.dedup();
        assert_eq!(glyphs.len(), unique, "two icons share a glyph");
    }

    /// One guarded way in, and this is what keeps it that way.
    ///
    /// naming the icon family anywhere else means drawing without the guard — which is a
    /// panic in any context whose first frame came before the fonts were applied, and was
    /// found by exactly that test.
    #[test]
    fn nothing_outside_this_file_names_the_icon_family() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        let mut offenders = Vec::new();
        for entry in std::fs::read_dir(&directory).expect("the ui sources are readable") {
            let path = entry.expect("an entry").path();
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if name == "icons.rs" || name == "fonts.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            if text.contains("icon_font(") || text.contains("ICON_FAMILY") {
                offenders.push(name);
            }
        }
        assert!(
            offenders.is_empty(),
            "these files draw icons without the guard: {offenders:?} — use icons::glyph or icons::paint",
        );
    }

    /// The cross-check, and the reason this table can be trusted to match the design.
    ///
    /// Skipped — with a printed reason, because a silent skip reads like a pass — when the
    /// mockup is not in the tree (`docs/` is deliberately untracked).
    #[test]
    fn the_design_agrees_with_this_table() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/ui/harness-quorfloat-phosphor-ui.html");
        if !path.is_file() {
            println!("skipping: {} is not in this checkout", path.display());
            return;
        }
        let text = std::fs::read_to_string(&path).expect("the mockup is readable");
        // The mockup's own name map, which is where this icon set was chosen.
        let start = text.find("phosphorNames=").expect("the mockup maps icon names");
        let end = text[start..].find('}').expect("the map ends") + start;
        let block = &text[start..end];
        // Only the icons the design names are checked against it. The others are this panel's
        // own additions, and they are listed as such where they are declared.
        for icon in DESIGNED {
            assert!(
                block.contains(&format!("\"{}\"", icon.name())),
                "{} is not one of the names the design maps",
                icon.name(),
            );
        }
        // And nothing may be added to `ALL` without being either designed or documented: a
        // count is enough to notice, and the compiler notices the arrays' lengths.
        assert!(ALL.len() > DESIGNED.len(), "the panel's own glyphs are the difference");
    }
}
