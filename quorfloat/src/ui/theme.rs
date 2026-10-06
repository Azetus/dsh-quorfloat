//! The design tokens: two palettes, one type scale, and the measurements in between.
//!
//! Every value here is taken from the design (`docs/ui/harness-quorfloat-phosphor-ui.html`)
//! rather than chosen by us, so that "does the panel look like the design" is a question
//! with an answer. Where the design states a value once and uses it in one place, it is a
//! constant here with that place named; where the design repeats a value, it is a token.
//!
//! **The palette is process state, set once per frame.** One panel, one theme: passing a
//! `&Palette` through every drawing function would be ceremony that no test benefits from,
//! and the alternative — reading the system theme inside every widget — would put the
//! decision in a dozen places instead of one. [`set_mode`] is the single writer, called
//! from the frame that just resolved the preference; everything else reads [`palette`].
//!
//! The design's own scale is not a strict 4-point grid (it uses 3, 5, 7, 9, 14, 17, 21 as
//! readily as 4, 8, 12), so this file does not invent one: it names the measurements the
//! design actually uses, and the names say where they belong.

use eframe::egui;
use std::sync::atomic::{AtomicU8, Ordering};

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

// ── the two palettes ────────────────────────────────────────────────────────────────

/// Which palette the panel is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The design's `light-dark(…, dark)` side.
    #[default]
    Dark,
    /// The design's `light-dark(light, …)` side.
    Light,
}

impl Mode {
    /// The single byte this mode is stored as.
    const fn as_u8(self) -> u8 {
        match self {
            Self::Dark => 0,
            Self::Light => 1,
        }
    }

    /// Read back what [`Mode::as_u8`] wrote.
    const fn from_u8(value: u8) -> Self {
        if value == 1 {
            Self::Light
        } else {
            Self::Dark
        }
    }

    /// The mode egui says the system is in.
    ///
    /// @param ctx - the render context, which knows what the platform reported.
    /// @returns the mode to draw in when the configuration expresses no preference.
    #[must_use]
    pub fn of_system(ctx: &egui::Context) -> Self {
        match ctx.theme() {
            egui::Theme::Light => Self::Light,
            egui::Theme::Dark => Self::Dark,
        }
    }
}

/// What the configuration says about the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    /// Follow the platform, which is what a panel floating over other apps should do.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl Preference {
    /// Read the preference from the environment.
    ///
    /// An unreadable or unknown value means [`Preference::System`]: a typo in an
    /// environment variable should not decide how the panel looks.
    ///
    /// @returns the configured preference.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_name(std::env::var("DSH_QUORFLOAT_THEME").ok().as_deref())
    }

    /// Read a preference from its name, which is also how the host will send it.
    ///
    /// @param name - `light`, `dark`, `system`, or anything else.
    /// @returns the matching preference, defaulting to following the system.
    #[must_use]
    pub fn from_name(name: Option<&str>) -> Self {
        match name.map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("light") => Self::Light,
            Some(value) if value.eq_ignore_ascii_case("dark") => Self::Dark,
            _ => Self::System,
        }
    }

    /// Resolve the preference against what the platform reports.
    ///
    /// @param self - the configured preference.
    /// @param ctx - the render context.
    /// @returns the mode to draw this frame in.
    #[must_use]
    pub fn resolve(self, ctx: &egui::Context) -> Mode {
        match self {
            Self::Light => Mode::Light,
            Self::Dark => Mode::Dark,
            Self::System => Mode::of_system(ctx),
        }
    }
}

/// One palette, with every role named after what it is for.
///
/// The first nine fields are the design's own tokens, in its order; the last four are the
/// roles the design has no opinion about because it draws no approval card — the panel
/// does, and they are chosen to sit in both palettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// `--q-bg`: the panel's own surface.
    pub bg: egui::Color32,
    /// `--q-soft`: hovered rows, buttons, and anything that recedes.
    pub soft: egui::Color32,
    /// `--q-text`: body text.
    pub text: egui::Color32,
    /// `--q-muted`: labels, metadata, anything secondary.
    pub muted: egui::Color32,
    /// `--q-line`: separators and 1px borders.
    pub line: egui::Color32,
    /// `--q-accent`: the one colour that means "this one".
    pub accent: egui::Color32,
    /// `--q-button`: the send button's fill.
    pub button: egui::Color32,
    /// `--q-onbutton`: what is drawn on [`Palette::button`].
    pub on_button: egui::Color32,
    /// `--q-shadow`: the colour both shadow layers are drawn in.
    pub shadow: egui::Color32,
    /// A success surface, for accepting something.
    pub good: egui::Color32,
    /// Success as text.
    pub good_text: egui::Color32,
    /// A refusal surface.
    pub bad: egui::Color32,
    /// Refusal or error as text.
    pub bad_text: egui::Color32,
    /// A warning as text: unreadable fonts, a handoff that needs the browser.
    pub warn_text: egui::Color32,
    /// The handoff banner's surface, which has to read as "this is not the panel".
    pub banner: egui::Color32,
}

impl Palette {
    /// The design's light side.
    pub const LIGHT: Self = Self {
        bg: rgb(0xfa, 0xfa, 0xf9),
        soft: rgb(0xf0, 0xf1, 0xf3),
        text: rgb(0x24, 0x26, 0x2c),
        muted: rgb(0x72, 0x75, 0x7d),
        line: rgb(0xe4, 0xe5, 0xe7),
        accent: rgb(0x50, 0x5c, 0xaf),
        button: rgb(0x30, 0x36, 0x43),
        on_button: rgb(0xff, 0xff, 0xff),
        shadow: rgba(0x20, 0x26, 0x38, 0x22),
        good: rgb(0x1f, 0x6f, 0x4a),
        good_text: rgb(0x1a, 0x5c, 0x3d),
        bad: rgb(0x9b, 0x2c, 0x2c),
        bad_text: rgb(0x8c, 0x27, 0x27),
        warn_text: rgb(0x8a, 0x63, 0x11),
        banner: rgb(0xf6, 0xef, 0xdd),
    };

    /// The design's dark side.
    pub const DARK: Self = Self {
        bg: rgb(0x24, 0x25, 0x29),
        soft: rgb(0x30, 0x32, 0x38),
        text: rgb(0xed, 0xed, 0xef),
        muted: rgb(0xa5, 0xa8, 0xb0),
        line: rgb(0x3b, 0x3d, 0x43),
        accent: rgb(0xa8, 0xb1, 0xff),
        button: rgb(0xdd, 0xdf, 0xe8),
        on_button: rgb(0x24, 0x26, 0x2c),
        shadow: rgba(0x00, 0x00, 0x00, 0x66),
        good: rgb(0x26, 0x5c, 0x3a),
        good_text: rgb(0x86, 0xcd, 0x9e),
        bad: rgb(0x68, 0x2a, 0x2e),
        bad_text: rgb(0xe2, 0x8c, 0x8c),
        warn_text: rgb(0xe4, 0xc4, 0x7a),
        banner: rgb(0x3a, 0x32, 0x1e),
    };

    /// The palette for a mode.
    ///
    /// @param mode - which one.
    /// @returns the palette.
    #[must_use]
    pub const fn of(mode: Mode) -> Self {
        match mode {
            Mode::Light => Self::LIGHT,
            Mode::Dark => Self::DARK,
        }
    }
}

/// A colour from the design's hex notation.
const fn rgb(r: u8, g: u8, b: u8) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

/// A colour with alpha, for the shadow tokens.
const fn rgba(r: u8, g: u8, b: u8, a: u8) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(r, g, b, a)
}

/// Which palette the next frame is drawn in.
static MODE: AtomicU8 = AtomicU8::new(Mode::Dark.as_u8());

/// Choose the palette for subsequent frames.
///
/// @param mode - the mode this frame resolved to.
pub fn set_mode(mode: Mode) {
    MODE.store(mode.as_u8(), Ordering::Relaxed);
}

/// The palette this frame is drawn in.
#[must_use]
pub fn palette() -> Palette {
    Palette::of(Mode::from_u8(MODE.load(Ordering::Relaxed)))
}

/// The panel's surface.
#[must_use]
pub fn bg() -> egui::Color32 {
    palette().bg
}

/// Hovered rows and receding surfaces.
#[must_use]
pub fn soft() -> egui::Color32 {
    palette().soft
}

/// Body text.
#[must_use]
pub fn text() -> egui::Color32 {
    palette().text
}

/// Labels and metadata.
#[must_use]
pub fn muted() -> egui::Color32 {
    palette().muted
}

/// Separators and borders.
#[must_use]
pub fn line() -> egui::Color32 {
    palette().line
}

/// The one colour that means "this one".
#[must_use]
pub fn accent() -> egui::Color32 {
    palette().accent
}

/// The send button's fill.
#[must_use]
pub fn button() -> egui::Color32 {
    palette().button
}

/// What is drawn on the send button.
#[must_use]
pub fn on_button() -> egui::Color32 {
    palette().on_button
}

/// A success surface, for accepting something.
#[must_use]
pub fn good() -> egui::Color32 {
    palette().good
}

/// Success as text.
#[must_use]
pub fn good_text() -> egui::Color32 {
    palette().good_text
}

/// A refusal surface.
#[must_use]
pub fn bad() -> egui::Color32 {
    palette().bad
}

/// Refusal or error as text.
#[must_use]
pub fn bad_text() -> egui::Color32 {
    palette().bad_text
}

/// A warning as text.
#[must_use]
pub fn warn_text() -> egui::Color32 {
    palette().warn_text
}

/// The handoff banner's surface.
#[must_use]
pub fn banner() -> egui::Color32 {
    palette().banner
}

// ── type ────────────────────────────────────────────────────────────────────────────

/// The design's base size: body text, and what every other size is relative to.
pub const TEXT_BODY: f32 = 14.0;

/// The brand line in the top bar (`--q-brand`, 12px at weight 500, 1.3px letter spacing).
pub const TEXT_BRAND: f32 = 12.0;

/// Labels, metadata, questions, menu rows, the clipboard line.
pub const TEXT_META: f32 = 12.0;

/// Footer hints and menu hints (`--q-footer`, `.q-pophint`).
pub const TEXT_SMALL: f32 = 11.0;

/// The composer, which is the panel's largest text on purpose (`textarea`, 20px).
pub const TEXT_COMPOSER: f32 = 20.0;

/// A heading inside the panel, between body and composer.
pub const TEXT_HEADING: f32 = 15.0;

/// Letter spacing on the brand line, from `letter-spacing:1.3px`.
pub const BRAND_LETTER_SPACING: f32 = 1.3;

/// How tall body text is laid out: `font:14px/1.6`.
pub const LINE_BODY: f32 = TEXT_BODY * 1.6;

/// The composer's line height: `line-height:1.5`.
pub const LINE_COMPOSER: f32 = TEXT_COMPOSER * 1.5;

/// An answer's line height: `line-height:1.85`, which is what makes a long answer
/// readable rather than dense.
pub const LINE_ANSWER: f32 = TEXT_BODY * 1.85;

/// How heavy a piece of text is.
///
/// The design uses 400, 500 and 600. egui has no bold, so each weight is a separate
/// family cut from the bundled variable font ([`crate::ui::fonts`] explains why the
/// unavailable ones fall back rather than panic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weight {
    /// Ordinary text.
    Regular,
    /// The brand line, "you" in a turn, a pressed picker.
    Medium,
    /// Reserved for the strongest emphasis the panel has.
    SemiBold,
}

/// The font to draw a piece of text with.
///
/// @param ctx - the context that will draw the text, which knows whether the weights were
///   installed.
/// @param weight - how heavy.
/// @param size - how large, in points.
/// @returns the font id, always naming a family that exists.
#[must_use]
pub fn font(ctx: &egui::Context, weight: Weight, size: f32) -> egui::FontId {
    egui::FontId::new(size, family(weight, crate::ui::fonts::has_weighted_families(ctx)))
}

/// The family for a weight, given whether the weighted cuts are available.
///
/// Pure, and separate from [`font`], because the interesting case is the one that cannot
/// happen on a developer's machine: with no bundled font there are no weighted families,
/// and epaint **panics** on a family that is not bound to any fonts. Falling back to
/// `Proportional` is therefore not politeness — it is the difference between a warning in
/// the panel and a crash at startup.
///
/// @param weight - how heavy.
/// @param weighted - whether the weights were registered.
/// @returns a family that is bound to fonts either way.
#[must_use]
pub fn family(weight: Weight, weighted: bool) -> egui::FontFamily {
    match (weight, weighted) {
        (Weight::Medium, true) => egui::FontFamily::Name(crate::ui::fonts::WEIGHT_MEDIUM.into()),
        (Weight::SemiBold, true) => egui::FontFamily::Name(crate::ui::fonts::WEIGHT_SEMIBOLD.into()),
        _ => egui::FontFamily::Proportional,
    }
}

/// A heading, for the session title and card titles.
/// The egui style the Markdown viewer draws with.
///
/// The viewer takes its sizes and colours from `ui.style()`, so a rendered answer would
/// otherwise arrive in egui's defaults — a different type scale, a different link colour, a
/// different code background — and the same answer would look like two different things
/// depending on whether it happened to contain Markdown.
///
/// @param base - the style in force, whose non-text choices are kept.
/// @returns a style whose text matches the panel's own.
#[must_use]
pub fn markdown_style(base: &egui::Style) -> egui::Style {
    let mut style = base.clone();
    style.visuals.override_text_color = Some(text());
    // Code draws on `extreme_bg_color`; without it, inline code is indistinguishable from the
    // words around it.
    style.visuals.extreme_bg_color = soft();
    style.visuals.hyperlink_color = accent();
    style.spacing.item_spacing = egui::vec2(0.0, 6.0);
    let body = egui::FontId::new(TEXT_BODY, egui::FontFamily::Proportional);
    let heading = egui::FontId::new(TEXT_HEADING, egui::FontFamily::Proportional);
    let small = egui::FontId::new(TEXT_META, egui::FontFamily::Proportional);
    style.text_styles = [
        (egui::TextStyle::Body, body.clone()),
        (egui::TextStyle::Monospace, egui::FontId::new(TEXT_BODY, egui::FontFamily::Monospace)),
        (egui::TextStyle::Button, body.clone()),
        (egui::TextStyle::Small, small.clone()),
        // A heading inside an answer is a subheading of the answer, not of the panel: none of
        // them may out-shout the composer, which is the largest text here by design.
        (egui::TextStyle::Heading, heading.clone()),
        (egui::TextStyle::Name("Heading2".into()), heading),
        (egui::TextStyle::Name("Heading3".into()), body),
        (egui::TextStyle::Name("Heading4".into()), small.clone()),
        (egui::TextStyle::Name("Heading5".into()), small.clone()),
        (egui::TextStyle::Name("Heading6".into()), small),
    ]
    .into();
    style
}

pub fn heading(ctx: &egui::Context, text_value: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text_value).font(font(ctx, Weight::SemiBold, TEXT_HEADING)).color(text())
}

/// Body text.
#[must_use]
pub fn body(ctx: &egui::Context, text_value: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text_value).font(font(ctx, Weight::Regular, TEXT_BODY)).color(text())
}

/// Secondary text: labels, metadata, counts.
#[must_use]
pub fn meta(ctx: &egui::Context, text_value: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text_value).font(font(ctx, Weight::Regular, TEXT_META)).color(muted())
}

// ── measurements ────────────────────────────────────────────────────────────────────

/// The window's corner radius (`border-radius:18px`).
pub const RADIUS_WINDOW: u8 = 18;

/// Menus and popovers (`--q-popover`, `border-radius:10px`).
pub const RADIUS_POPOVER: u8 = 10;

/// The send button (`--q-submit`, `border-radius:9px`).
pub const RADIUS_SUBMIT: u8 = 9;

/// Icon buttons like close and settings (`--q-close`, `border-radius:7px`).
pub const RADIUS_ICON_BUTTON: u8 = 7;

/// Pickers and menu rows (`--q-picker`, `border-radius:6px`).
pub const RADIUS_PICKER: u8 = 6;

/// The smallest buttons: "remove", the pin in a menu row, `radius:5px`.
pub const RADIUS_SMALL_BUTTON: u8 = 5;

/// The keyboard hints in the footer (`--q-footer kbd`, `border-radius:4px`).
pub const RADIUS_KBD: u8 = 4;

/// Space above a turn's separator, and below it — the design's `.q-turn + .q-turn`.
pub const TURN_GAP_ABOVE: f32 = 20.0;
/// @see TURN_GAP_ABOVE
pub const TURN_GAP_BELOW: f32 = 18.0;
/// Between a question and the answer it belongs to (`.q-question` margin-bottom).
pub const QUESTION_GAP: f32 = 14.0;
/// Between an answer and its bar (`.q-answerbar` margin-top).
pub const ANSWER_BAR_GAP: f32 = 16.0;

/// The window's border: `1px solid var(--q-line)`.
pub const BORDER: f32 = 1.0;

/// The shadow that makes the panel float: `0 16px 42px` plus `0 3px 8px`.
pub const SHADOW_OFFSET: [i8; 2] = [0, 16];
/// The wide, soft layer of the float shadow.
pub const SHADOW_BLUR: u8 = 42;
/// The near layer, which gives the edge definition.
pub const SHADOW_NEAR_OFFSET: [i8; 2] = [0, 3];
/// The near layer's blur.
pub const SHADOW_NEAR_BLUR: u8 = 8;

/// Icon buttons in the top bar (`--q-close`, 28×28).
pub const ICON_BUTTON: f32 = 28.0;

/// The send button (`--q-submit`, 34×34).
pub const SEND_BUTTON: f32 = 34.0;

/// Icons in a picker (`--q-picker > svg:first-child`, 14px).
pub const ICON_PICKER: f32 = 14.0;

/// The disclosure carets (`--q-chevron`, 12px).
pub const ICON_CHEVRON: f32 = 12.0;

/// The default icon box (`--q-icon`, 16px).
pub const ICON: f32 = 16.0;

/// Space inside the window's top bar: `padding:12px 17px 0 22px`.
pub const PAD_TOP: egui::Margin = egui::Margin { left: 22, right: 17, top: 12, bottom: 0 };

/// Space around the composer: `padding:21px 22px 14px`.
pub const PAD_COMPOSER: egui::Margin = egui::Margin { left: 22, right: 22, top: 21, bottom: 14 };

/// Space inside the conversation: `padding:20px 24px 22px`.
pub const PAD_THREAD: egui::Margin = egui::Margin { left: 24, right: 24, top: 20, bottom: 22 };

/// Space inside the footer: `padding:8px 15px 8px 22px`.
pub const PAD_FOOTER: egui::Margin = egui::Margin { left: 22, right: 15, top: 8, bottom: 8 };

/// Gap between the parts of a row (`gap:14px` in the top bar, `gap:10px` in the footer).
pub const GAP: f32 = 10.0;

/// A tighter gap, for icons against their labels (`gap:6px`).
/// The horizontal padding of the card section: the composer's, because a card is about what the
/// composer is doing — an approval is a message that cannot be sent until it is answered — and a
/// box that starts where the search icon starts reads as belonging to it.
pub const PAD_CARDS: egui::Margin = egui::Margin {
    left: 22,
    right: 22,
    top: 0,
    bottom: 0,
};

/// Inside a card (an approval or a question): the same family as the composer's padding,
/// smaller because a card is a box inside the panel rather than the panel itself.
pub const PAD_CARD: egui::Margin = egui::Margin::same(14);

/// Between the rows of a card: its title, its reason, its options, its buttons.
pub const GAP_CARD: f32 = 10.0;

/// A card's surface: the panel's own soft fill, the popover's radius, the card's padding.
///
/// Approvals and questions are the panel's boxes-inside-the-panel, so they are built from the
/// same three choices every other surface here is built from rather than from numbers of their
/// own — which is what made them look like they came from a different application.
///
/// @returns the frame to draw a card in.
#[must_use]
pub fn card_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(soft())
        .corner_radius(egui::CornerRadius::same(RADIUS_POPOVER))
        .inner_margin(PAD_CARD)
}

/// A card's title.
///
/// @param ctx - for the panel's own font.
/// @param text_value - the title.
/// @returns the styled text.
#[must_use]
pub fn card_title(ctx: &egui::Context, text_value: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text_value)
        .font(font(ctx, Weight::Medium, TEXT_BODY))
        .color(text())
}

/// The panel's primary button: one filled action per view, the design's `--q-button` on
/// `--q-on-button`, at the send button's radius.
///
/// Every filled button in the panel is this one. A second filled button in the same view — a
/// green "allow" beside a red "reject", say — is what the design's vocabulary does not have:
/// it draws exactly one filled control per view and lets the rest be surfaces or text.
///
/// @param ctx - for the panel's own font.
/// @param label - what the button says.
/// @returns the button, ready to add to a layout.
#[must_use]
pub fn primary_button(ctx: &egui::Context, label: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label.to_owned())
            .font(font(ctx, Weight::Medium, TEXT_BODY))
            .color(on_button()),
    )
    .fill(button())
    .corner_radius(egui::CornerRadius::same(RADIUS_SUBMIT))
    .min_size(egui::vec2(0.0, 30.0))
}

/// The panel's secondary button: a surface with a hairline, for the other choices in a view.
///
/// @param ctx - for the panel's own font.
/// @param label - what the button says.
/// @returns the button, ready to add to a layout.
#[must_use]
pub fn secondary_button(ctx: &egui::Context, label: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(label.to_owned())
            .font(font(ctx, Weight::Regular, TEXT_BODY))
            .color(text()),
    )
    .fill(soft())
    .stroke(egui::Stroke::new(BORDER, line()))
    .corner_radius(egui::CornerRadius::same(RADIUS_SUBMIT))
    .min_size(egui::vec2(0.0, 30.0))
}

/// The surface of one option inside a card: the picker row's shape, without its behaviour.
///
/// A question's choices are read-only here, but they must still look like the choices they are —
/// and like the rows of the picker, which is where the user will meet the same options.
///
/// @returns the frame to draw an option in.
#[must_use]
pub fn option_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(bg())
        .stroke(egui::Stroke::new(BORDER, line()))
        .corner_radius(egui::CornerRadius::same(RADIUS_PICKER))
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 8,
            bottom: 8,
        })
}

pub const GAP_TIGHT: f32 = 6.0;

/// The closest two things get (`gap:3px` between the tools).
pub const GAP_CLOSE: f32 = 3.0;

/// The composer's spacing around the input (`gap:14px`).
pub const GAP_COMPOSER: f32 = 14.0;

/// Between one turn and the next (`margin-top:20px`).
pub const GAP_TURN: f32 = 20.0;

/// How much room the panel leaves around itself for its own shadow.
///
/// A transparent window means the shadow is ours to draw, and a shadow drawn outside the
/// panel's rect needs somewhere to be: 42px of blur plus the 16px offset is the widest
/// reach the design asks for, and the panel insets itself by that much so the window's
/// edge never cuts the shadow off.
pub const SHADOW_ROOM_SIDE: i8 = 34;
/// The room above the panel: the shadow reaches down, not up, so this is only the blur's
/// first few pixels.
pub const SHADOW_ROOM_TOP: i8 = 20;
/// The room below the panel, where the shadow is at its longest: the far layer is drawn
/// `offset` (16) plus half its blur (21) below the panel's edge, so 34 was three pixels
/// short of what it needs and the shadow was clipped flat along the bottom.
pub const SHADOW_ROOM_BOTTOM: i8 = 40;

/// The far layer of the float shadow: `0 16px 42px`.
#[must_use]
pub fn shadow_far() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: SHADOW_OFFSET,
        blur: SHADOW_BLUR,
        spread: 0,
        color: palette().shadow,
    }
}

/// The near layer: `0 3px 8px`, which is what gives the panel's edge its definition.
#[must_use]
pub fn shadow_near() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: SHADOW_NEAR_OFFSET,
        blur: SHADOW_NEAR_BLUR,
        spread: 0,
        color: palette().shadow,
    }
}

/// The popover's shadow: `0 10px 30px`, the design's own for anything that floats above
/// the panel.
#[must_use]
pub fn shadow_popover() -> egui::epaint::Shadow {
    egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: palette().shadow }
}

/// The frame every popover is drawn in: the design's `--q-popover`.
#[must_use]
pub fn popover_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(bg())
        .corner_radius(egui::CornerRadius::same(RADIUS_POPOVER))
        .stroke(egui::Stroke::new(BORDER, line()))
        .shadow(shadow_popover())
        .inner_margin(egui::Margin::same(6))
}

/// How wide a popover is (`--q-popover`, 276px; the small one is 218px).
pub const POPOVER_WIDTH: f32 = 276.0;

/// Space between an option's rows (`padding:5px 7px` in the design's picker).
pub const ROW_PADDING: egui::Margin = egui::Margin { left: 9, right: 9, top: 8, bottom: 8 };

/// How long the panel takes to appear or leave (`--q-speed`, 180ms).
pub const SPEED: f32 = 0.18;

/// How long the conversation takes to unfold (`220ms ease-out`).
pub const SPEED_EXPAND: f32 = 0.22;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_designs_own_values_are_the_ones_in_this_file() {
        // Straight from `docs/ui/harness-quorfloat-phosphor-ui.html`. If the design changes
        // these, this test is where the change has to be acknowledged.
        assert_eq!(Palette::LIGHT.bg, egui::Color32::from_rgb(0xfa, 0xfa, 0xf9));
        assert_eq!(Palette::LIGHT.accent, egui::Color32::from_rgb(0x50, 0x5c, 0xaf));
        assert_eq!(Palette::LIGHT.line, egui::Color32::from_rgb(0xe4, 0xe5, 0xe7));
        assert_eq!(Palette::DARK.bg, egui::Color32::from_rgb(0x24, 0x25, 0x29));
        assert_eq!(Palette::DARK.accent, egui::Color32::from_rgb(0xa8, 0xb1, 0xff));
        assert_eq!(Palette::DARK.soft, egui::Color32::from_rgb(0x30, 0x32, 0x38));
        assert_eq!(RADIUS_WINDOW, 18);
        assert_eq!(TEXT_COMPOSER, 20.0);
        assert_eq!(SPEED, 0.18);
    }

    #[test]
    fn the_two_palettes_are_surfaces_the_text_can_be_read_on() {
        // Not a contrast audit — just the difference that would make the panel unusable if
        // it were wrong: in the light palette the text is darker than its surface, and in
        // the dark palette it is lighter.
        let light = Palette::LIGHT;
        let dark = Palette::DARK;
        let luminance = |colour: egui::Color32| {
            u32::from(colour.r()) + u32::from(colour.g()) + u32::from(colour.b())
        };
        assert!(luminance(light.text) < luminance(light.bg), "dark text on a light panel");
        assert!(luminance(dark.text) > luminance(dark.bg), "light text on a dark panel");
        assert!(luminance(light.muted) < luminance(light.bg), "muted text is still readable");
        assert!(luminance(dark.muted) > luminance(dark.bg), "and so is the dark one's");
        // The send button has to be visible against the panel in both, and its glyph has to
        // be visible against the button.
        for palette in [light, dark] {
            assert!(palette.button != palette.bg, "the send button is not the panel");
            assert!(
                luminance(palette.on_button).abs_diff(luminance(palette.button)) > 120,
                "and what is drawn on it stands out",
            );
        }
    }

    #[test]
    fn a_preference_is_read_the_way_the_host_will_send_it() {
        assert_eq!(Preference::from_name(Some("light")), Preference::Light);
        assert_eq!(Preference::from_name(Some(" Dark ")), Preference::Dark);
        assert_eq!(Preference::from_name(Some("system")), Preference::System);
        // A typo decides nothing: following the platform is the only safe default for a
        // panel that floats over other people's applications.
        assert_eq!(Preference::from_name(Some("chartreuse")), Preference::System);
        assert_eq!(Preference::from_name(None), Preference::System);
    }

    #[test]
    fn an_explicit_preference_ignores_what_the_platform_says() {
        // The resolve step needs a context, but the interesting half is that Light and Dark
        // do not consult it at all.
        let ctx = egui::Context::default();
        assert_eq!(Preference::Light.resolve(&ctx), Mode::Light);
        assert_eq!(Preference::Dark.resolve(&ctx), Mode::Dark);
    }

    #[test]
    fn set_mode_changes_what_everyone_reads() {
        set_mode(Mode::Light);
        assert_eq!(palette().bg, Palette::LIGHT.bg);
        assert_eq!(bg(), Palette::LIGHT.bg);
        assert_eq!(text(), Palette::LIGHT.text);
        set_mode(Mode::Dark);
        assert_eq!(palette().bg, Palette::DARK.bg);
        assert_eq!(accent(), Palette::DARK.accent);
    }

    #[test]
    fn a_missing_font_falls_back_rather_than_naming_a_family_that_does_not_exist() {
        // epaint panics on a family bound to no fonts, so this is the difference between a
        // warning in the panel and a crash at startup on a machine without the bundled font.
        for weight in [Weight::Regular, Weight::Medium, Weight::SemiBold] {
            assert_eq!(family(weight, false), egui::FontFamily::Proportional);
        }
        assert_eq!(family(Weight::Regular, true), egui::FontFamily::Proportional);
        assert_eq!(
            family(Weight::Medium, true),
            egui::FontFamily::Name(crate::ui::fonts::WEIGHT_MEDIUM.into()),
        );
        assert_eq!(
            family(Weight::SemiBold, true),
            egui::FontFamily::Name(crate::ui::fonts::WEIGHT_SEMIBOLD.into()),
        );
    }

    #[test]
    fn the_type_scale_is_ordered_the_way_the_design_uses_it() {
        assert!(TEXT_SMALL < TEXT_META);
        assert_eq!(TEXT_META, TEXT_BRAND);
        assert!(TEXT_META < TEXT_BODY);
        assert!(TEXT_BODY < TEXT_HEADING);
        assert!(TEXT_HEADING < TEXT_COMPOSER);
        // Line heights follow the design's ratios, and an answer is the roomiest of them.
        assert!(LINE_ANSWER > LINE_BODY);
        assert!(LINE_COMPOSER > LINE_BODY);
        assert!((LINE_ANSWER - 25.9).abs() < 0.05, "14px at 1.85");
    }
}
