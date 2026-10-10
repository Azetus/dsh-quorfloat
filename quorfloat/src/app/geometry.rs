//! Where the panel was last time, so it comes back there.
//!
//! Window geometry is presentation state, which the design assigns to this process rather
//! than to the host: the panel is a thing the user moves around,
//! and a panel that jumps back to a default corner after every restart is a panel they
//! have to re-place every time the host restarts it. The settings system will own this in
//! P3; until then it is a small file this process reads and writes.
//!
//! Three rules, the same ones the diagnostics follow for the same reason — a file that can
//! break the product is worse than no file:
//!
//! - **Nothing is load-bearing.** A missing, unreadable, unparseable or absurd file means
//!   "no remembered position", not an error.
//! - **Nothing is written without a place to write it.** The path comes from the
//!   environment, with one sensible default under the user's own config directory.
//! - **Garbage is refused, not clamped.** A position that is on no attached display is worse
//!   than no position: the hotkey would reveal a panel the user cannot see.
//!
//! The third rule is why this module now takes the displays as an argument. A remembered
//! coordinate is a **display-arrangement** coordinate: the same numbers mean a different
//! physical spot once a monitor is unplugged, and a coordinate that belonged to a monitor that
//! is gone is on no display at all. The real value lives in the user's file
//! (`{"x":402.0,"y":11276.0}` on a 3024x1964 built-in display), so the refusal cannot be a
//! plausibility check on the number itself — the shell asks the platform which displays exist
//! and hands the answer here as plain rectangles.
//!
//! The decisions below are pure functions over those rectangles, so every boundary — a corner
//! exactly on an edge, a second display at a negative origin, a platform that reports no
//! displays at all — is a unit test and not a hardware arrangement.

use std::path::PathBuf;

/// A remembered window position.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Geometry {
    /// Where the window's top-left corner was, in screen coordinates.
    position: Option<(f32, f32)>,
}

impl Geometry {
    /// The remembered position, if one was remembered and it is sane.
    #[must_use]
    pub fn position(&self) -> Option<(f32, f32)> {
        self.position
    }

    /// Remember a position, unless it is not a position a window can have.
    ///
    /// @param position - the window's top-left corner.
    /// @returns whether it was worth remembering.
    pub fn remember(&mut self, position: (f32, f32)) -> bool {
        if !plausible(position) {
            return false;
        }
        self.position = Some(position);
        true
    }

    /// Read the remembered position.
    ///
    /// @param path - where it would be. Missing or unreadable means nothing is remembered.
    /// @returns what was read, with anything implausible dropped.
    #[must_use]
    pub fn load(path: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return Self::default();
        };
        let position = value
            .get("x")
            .and_then(serde_json::Value::as_f64)
            .zip(value.get("y").and_then(serde_json::Value::as_f64))
            .map(|(x, y)| (x as f32, y as f32))
            .filter(|position| plausible(*position));
        Self { position }
    }

    /// Write the remembered position, if there is one.
    ///
    /// Failure is ignored: not being able to remember where the user put the panel is a
    /// smaller problem than refusing to run because of it.
    ///
    /// @param path - where to write. Its directory is created when needed.
    pub fn save(&self, path: &std::path::Path) {
        let Some((x, y)) = self.position else { return };
        if let Some(directory) = path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }
        let body = serde_json::json!({ "x": x, "y": y });
        let _ = std::fs::write(path, body.to_string());
    }
}

/// A display's rectangle, in the physical coordinates the platform reports windows in.
///
/// Plain numbers rather than a `tauri::Monitor`, because every decision made from it has to be
/// testable where there is no second screen (and on CI, where there is no screen at all): the
/// shell converts the platform's answer into this once, at the edge, and the rules below never
/// touch a window handle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    /// Left edge. Negative is normal: a display placed left of the primary one starts there.
    pub x: f32,
    /// Top edge. Negative likewise for a display placed above the primary one.
    pub y: f32,
    /// Width in physical pixels.
    pub width: f32,
    /// Height in physical pixels.
    pub height: f32,
}

impl ScreenRect {
    /// A display's rectangle.
    ///
    /// @param x - left edge.
    /// @param y - top edge.
    /// @param width - width in physical pixels.
    /// @param height - height in physical pixels.
    /// @returns the rectangle.
    #[must_use]
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// Whether a point is on this display.
    ///
    /// Both far edges count as on it: the point tested is a window's top-left corner, and a
    /// corner that lands exactly on the seam between two displays is on both of them — refusing
    /// it would move a panel the user placed deliberately.
    ///
    /// @param point - a point in the same physical coordinates.
    /// @returns whether the point is inside, edges included.
    #[must_use]
    pub fn contains(self, (x, y): (f32, f32)) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// Where on a display the panel opens.
///
/// The host's own vocabulary (`WindowAnchor` in `src/config.ts`), spelled the same way, so the
/// two halves of the product name the same four placements and a value cannot be understood in
/// one half and silently ignored in the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    /// Horizontally centred against the display's top edge.
    TopCenter,
    /// The display's top-left corner.
    TopLeft,
    /// The display's top-right corner.
    TopRight,
    /// The middle of the display.
    ///
    /// The default, and the reason it is not `TopCenter`: the panel is the thing the hotkey
    /// summons, so it has to be somewhere the user is already looking, not tucked against an
    /// edge of a screen they may not be facing. It is also the anchor the refusal path falls
    /// back to, which is why it must be a place that is *certainly* visible.
    #[default]
    Center,
}

impl Anchor {
    /// Read the host's spelling.
    ///
    /// @param name - the configuration value, e.g. `top-center`.
    /// @returns the anchor, or `None` for a name this build does not know (the caller keeps the
    ///   default: an unknown spelling must not move the panel somewhere arbitrary).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "top-center" => Some(Self::TopCenter),
            "top-left" => Some(Self::TopLeft),
            "top-right" => Some(Self::TopRight),
            "center" => Some(Self::Center),
            _ => None,
        }
    }

    /// The host's spelling, for a marker line.
    ///
    /// @returns the name as `src/config.ts` writes it, so a log line and a config file agree.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::TopCenter => "top-center",
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::Center => "center",
        }
    }
}

/// Whether a point is on any of the displays the platform reported.
///
/// An empty list is an answer: the caller asked which displays exist and was told none, so no
/// point is on one. That is the conservative reading on purpose — "I cannot verify this
/// coordinate" and "this coordinate is on no display" both mean *do not trust it*, and the
/// alternative (treating an unreported display as a licence to restore anything) is the failure
/// this module exists to prevent.
///
/// @param point - the window's top-left corner.
/// @param displays - every display the platform reports.
/// @returns whether the point is inside at least one of them.
#[must_use]
pub fn on_a_display(point: (f32, f32), displays: &[ScreenRect]) -> bool {
    displays.iter().any(|display| display.contains(point))
}

/// Where a window of `size` opens on `display` under `anchor`.
///
/// The caller passes a display's **work area**, not its raw rectangle: the top edge of a screen
/// is the macOS menu bar and the Windows taskbar, and a panel anchored to "the top" of the frame
/// would open underneath it — the panel is an undecorated accessory with no title bar, so
/// nothing would say it was there. The work area is the part of the display a window can
/// actually occupy, which is also the honest meaning of "the centre of the display".
///
/// @param display - the display's work area, in physical pixels.
/// @param size - the window's size in the same physical pixels.
/// @param anchor - where on that display to place it.
/// @returns the window's top-left corner.
#[must_use]
pub fn anchored(display: ScreenRect, size: (f32, f32), anchor: Anchor) -> (f32, f32) {
    let centred_x = display.x + (display.width - size.0) / 2.0;
    let centred_y = display.y + (display.height - size.1) / 2.0;
    match anchor {
        Anchor::TopLeft => (display.x, display.y),
        Anchor::TopCenter => (centred_x, display.y),
        Anchor::TopRight => (display.x + display.width - size.0, display.y),
        Anchor::Center => (centred_x, centred_y),
    }
}

/// Where the panel opens, decided from the displays that exist *right now*.
///
/// One variant per outcome the shell has to tell apart, including the two that carry no
/// position: the window code has to say what happened in the marker, and "the remembered
/// position was refused" must not have to be inferred from a `None`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    /// The remembered position is on a display and is used as it stands.
    Restored((f32, f32)),
    /// Nothing was remembered, so the anchor opens the panel on the primary display.
    Anchored((f32, f32)),
    /// The remembered position is on no display, so the anchor's position replaces it.
    ///
    /// Both coordinates are carried: the refusal line has to name what was dropped and what was
    /// done instead, and neither is recoverable afterwards.
    Replaced {
        /// The remembered position that was on no display.
        rejected: (f32, f32),
        /// The anchor's position, which is used.
        position: (f32, f32),
    },
    /// The remembered position is on no display and there was no display to anchor on, so the
    /// platform's own placement stands — the one placement that is on a display by construction.
    Refused((f32, f32)),
    /// Nothing was remembered and there was no display to anchor on.
    Platform,
}

impl Placement {
    /// The position to apply, if the decision reached one.
    ///
    /// @returns the top-left corner to ask the platform for, or `None` to leave the window
    ///   where the platform put it.
    #[must_use]
    pub fn position(self) -> Option<(f32, f32)> {
        match self {
            Self::Restored(position) | Self::Anchored(position) => Some(position),
            Self::Replaced { position, .. } => Some(position),
            Self::Refused(_) | Self::Platform => None,
        }
    }

    /// The one marker line this decision deserves.
    ///
    /// The wording lives here, beside the rule that produced it, for two reasons: the sentence a
    /// user reads and the decision cannot drift apart, and the line that matters most — the
    /// refusal, which names the coordinate that was dropped — is a test instead of a claim. It is
    /// written once per startup, never per frame.
    ///
    /// @param anchor - the configured fallback, named so a log line and a config file agree.
    /// @returns the line to write, or `None` when there is nothing to add: a window the platform
    ///   placed itself was already introduced by the line written before the window existed.
    #[must_use]
    pub fn note(self, anchor: Anchor) -> Option<String> {
        match self {
            Self::Restored((x, y)) => Some(format!("window restored at {x:.0},{y:.0}")),
            Self::Anchored((x, y)) => {
                Some(format!("window anchored at {x:.0},{y:.0} ({})", anchor.name()))
            }
            Self::Replaced { rejected: (rx, ry), position: (x, y) } => Some(format!(
                "window position rejected: {rx:.0},{ry:.0} is on no attached display; anchored at {x:.0},{y:.0} ({})",
                anchor.name(),
            )),
            Self::Refused((rx, ry)) => Some(format!(
                "window position rejected: {rx:.0},{ry:.0} is on no attached display; no display to anchor on",
            )),
            Self::Platform => None,
        }
    }
}

/// Decide where the panel opens, given the displays that exist now.
///
/// The rule in one line: **a remembered position wins unless it is on no attached display**, and
/// the anchor is where the panel goes instead. The failure it prevents: a coordinate recorded
/// while an external monitor was attached (`{"x":402.0,"y":11276.0}`)
/// outlives that monitor, the window is created there, and the hotkey and the tray's "show" then
/// focus a panel that is on no screen: both look like they did nothing.
///
/// **A window that merely hangs off an edge is honoured.** Only the window's own corner is
/// tested, so a panel whose body crosses the seam between two displays — or overhangs the outer
/// edge of one, as a hand-placed window does — is still "on a display" and is left alone. The
/// rule is exactly "the corner is on no attached display", no more.
///
/// @param remembered - the position from the geometry file, if it holds one.
/// @param displays - every display the platform reports.
/// @param primary - the primary display's work area, if the platform named one.
/// @param size - the opening window's size in the displays' physical pixels.
/// @param anchor - where to open when there is nothing to restore.
/// @returns the decision, including the coordinate that was refused.
#[must_use]
pub fn placement(
    remembered: Option<(f32, f32)>,
    displays: &[ScreenRect],
    primary: Option<ScreenRect>,
    size: (f32, f32),
    anchor: Anchor,
) -> Placement {
    let refused = remembered.filter(|point| !on_a_display(*point, displays));
    let anchored_fallback = || primary.map(|display| anchored(display, size, anchor));
    if let Some(rejected) = refused {
        return match anchored_fallback() {
            Some(position) => Placement::Replaced { rejected, position },
            None => Placement::Refused(rejected),
        };
    }
    match remembered {
        Some(position) => Placement::Restored(position),
        None => match anchored_fallback() {
            Some(position) => Placement::Anchored(position),
            None => Placement::Platform,
        },
    }
}

/// Where the geometry file lives when the environment does not name one.
///
/// The user's own config directory, because that is what this is: a desktop application
/// remembering where its window was. `DSH_QUORFLOAT_WINDOW_STATE` overrides it, which is
/// how the development harness keeps its state inside its throwaway home.
///
/// @returns the path, or `None` when there is no home to put it in.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())?;
    Some(PathBuf::from(home).join(".dsh-quorfloat").join("window.json"))
}

/// The path this process remembers its window position in.
///
/// @returns the environment's choice when it names one, otherwise the default.
#[must_use]
pub fn path_from_env() -> Option<PathBuf> {
    let named = std::env::var_os("DSH_QUORFLOAT_WINDOW_STATE")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    named.or_else(default_path)
}

/// How long a position has to hold still before it is written.
///
/// A drag produces a new position on every frame, and writing a file on every frame of a
/// drag is a file written sixty times a second for a value the user is still choosing.
/// One second after they let go is soon enough to survive a crash, and far enough from the
/// pointer to be a decision rather than a sample.
const SETTLE_MS: i64 = 1000;

/// The remembered position and the file it lives in.
///
/// Owns the when-to-write policy so that the window code does not have to: the panel's job
/// is to report where it is, and this decides whether that is worth a write.
#[derive(Debug, Clone, Default)]
pub struct WindowState {
    /// What was remembered at startup.
    geometry: Geometry,
    /// Where to remember it, or nowhere when there is no home to write to.
    path: Option<PathBuf>,
    /// The position the file currently holds, if it holds one.
    ///
    /// Tracked separately from the timestamp because "has this been written?" and "was that
    /// long enough ago?" are different questions: a drag that ends inside the settle window
    /// has a position worth writing and no time to wait for.
    written: Option<(f32, f32)>,
    /// When the last write happened, for the settling rule.
    ///
    /// `None` rather than `0` because zero is a real millisecond value: treating "never
    /// written" as "written at the epoch" makes the very first observation look settled.
    written_at: Option<i64>,
}

impl WindowState {
    /// Read the remembered position, if there is one to read.
    #[must_use]
    pub fn load() -> Self {
        let path = path_from_env();
        let geometry = path.as_deref().map(Geometry::load).unwrap_or_default();
        Self { geometry, path, written: None, written_at: None }
    }

    /// The position to open at, if one was remembered.
    #[must_use]
    pub fn position(&self) -> Option<(f32, f32)> {
        self.geometry.position()
    }

    /// What was asked of the platform at startup, kept for the first-sighting line.
    #[must_use]
    pub fn requested(&self) -> Option<(f32, f32)> {
        self.geometry.position()
    }

    /// Note where the window is now, and write it once it has settled.
    ///
    /// A position on no attached display is not remembered at all. This is the second half of
    /// the refusal above: a coordinate that belonged to a display which is no longer attached
    /// must not be handed to the next start, because that is how a bad value becomes a permanent
    /// one. The read path distrusts such a coordinate anyway (see [`placement`]); not writing it
    /// is how the file stops carrying it in the first place.
    ///
    /// @param position - the window's current top-left corner.
    /// @param displays - the displays the platform reports right now.
    /// @param now - caller-local time in milliseconds.
    /// @returns whether a write happened, for the caller's log line.
    pub fn observe(&mut self, position: (f32, f32), displays: &[ScreenRect], now: i64) -> bool {
        if !on_a_display(position, displays) {
            return false;
        }
        if !self.geometry.remember(position) {
            return false;
        }
        if self.written == Some(position) {
            return false;
        }
        if self.written_at.is_some_and(|written| now - written < SETTLE_MS) {
            return false;
        }
        self.write(now)
    }

    /// Write what is remembered, whatever the settling rule says.
    ///
    /// For the end of a session: the user may have dragged the panel and quit within the
    /// settle window, and that move is as real as any other.
    ///
    /// The display check is repeated here rather than trusted to [`Self::observe`], because
    /// what is written may have been *loaded* rather than observed: a file whose position was
    /// refused at startup would otherwise be rewritten at exit, handing the next start the
    /// coordinate the user has just been rescued from.
    ///
    /// @param displays - the displays the platform reports right now.
    /// @param now - caller-local time in milliseconds.
    /// @returns whether anything was written.
    pub fn flush(&mut self, displays: &[ScreenRect], now: i64) -> bool {
        let Some(position) = self.geometry.position() else {
            return false;
        };
        if self.written == Some(position) || !on_a_display(position, displays) {
            return false;
        }
        self.write(now)
    }

    /// Write what is remembered.
    ///
    /// @returns whether anything was written: "nowhere to write" is not a write, and a
    ///   caller that logs one would be logging a file that does not exist.
    fn write(&mut self, now: i64) -> bool {
        let Some(path) = &self.path else { return false };
        self.geometry.save(path);
        self.written = self.geometry.position();
        self.written_at = Some(now);
        true
    }
}

/// Whether a position is one a window could actually be at, by its numbers alone.
///
/// Deliberately loose, and it is only the first of two gates: negative coordinates are normal on
/// a multi-monitor desktop, so nothing here may reject them. What it rejects is the shapes that
/// come from a truncated write or a hand-edited file — NaN, infinities, and coordinates so large
/// that no desktop has them. Whether the number is on a display is a different question, answered
/// against the platform's display list by [`placement`] and [`WindowState::observe`]; this one
/// runs where no platform call is possible (reading the file), and must not pretend to know.
fn plausible((x, y): (f32, f32)) -> bool {
    const LIMIT: f32 = 32_000.0;
    x.is_finite() && y.is_finite() && x.abs() < LIMIT && y.abs() < LIMIT
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch path that does not exist yet.
    fn scratch(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("quorfloat-geometry-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// The one display most tests run on: a 1920x1080 built-in screen at the origin.
    fn display() -> ScreenRect {
        ScreenRect::new(0.0, 0.0, 1920.0, 1080.0)
    }

    /// The display list a machine with one screen reports.
    fn one_display() -> Vec<ScreenRect> {
        vec![display()]
    }

    /// A 3024x1964 built-in display with a remembered coordinate from a desk where an external
    /// monitor was attached above it, and the primary display's work area (menu bar excluded, as
    /// macOS reports it).
    fn measured() -> (Vec<ScreenRect>, Option<ScreenRect>, (f32, f32)) {
        let built_in = ScreenRect::new(0.0, 0.0, 3024.0, 1964.0);
        let work_area = ScreenRect::new(0.0, 25.0, 3024.0, 1914.0);
        (vec![built_in], Some(work_area), (402.0, 11276.0))
    }

    #[test]
    fn a_remembered_position_survives_a_round_trip() {
        let path = scratch("round");
        let mut geometry = Geometry::default();
        assert!(geometry.remember((120.0, 64.0)));
        geometry.save(&path);

        assert_eq!(Geometry::load(&path).position(), Some((120.0, 64.0)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nothing_is_remembered_until_something_is() {
        let mut geometry = Geometry::default();
        assert_eq!(geometry.position(), None);
        assert!(geometry.remember((-1200.0, 300.0)), "a second screen is to the left");
        assert_eq!(geometry.position(), Some((-1200.0, 300.0)));
    }

    #[test]
    fn a_missing_file_means_no_position_rather_than_an_error() {
        assert_eq!(Geometry::load(&scratch("missing")).position(), None);
    }

    #[test]
    fn a_corrupt_file_is_ignored() {
        // A half-written file is what a kill mid-save looks like, and it must cost the user
        // nothing but a default position.
        let path = scratch("corrupt");
        std::fs::write(&path, "{ this is not json").expect("write");
        assert_eq!(Geometry::load(&path).position(), None);

        std::fs::write(&path, r#"{"x": 10}"#).expect("write");
        assert_eq!(Geometry::load(&path).position(), None, "half a position is no position");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_absurd_position_is_refused_rather_than_clamped() {
        // Restoring one of these hides the panel where no screen is, and the hotkey would
        // then reveal it nowhere.
        for bad in [(f32::NAN, 0.0), (0.0, f32::INFINITY), (1.0e9, 0.0), (0.0, -1.0e9)] {
            let mut geometry = Geometry::default();
            assert!(!geometry.remember(bad), "{bad:?} is not a place for a window");
            assert_eq!(geometry.position(), None);
        }

        let path = scratch("absurd");
        std::fs::write(&path, r#"{"x": 99999999, "y": 0}"#).expect("write");
        assert_eq!(Geometry::load(&path).position(), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unwritable_path_is_survivable() {
        let mut geometry = Geometry::default();
        geometry.remember((10.0, 10.0));
        geometry.save(std::path::Path::new("/definitely/not/a/real/directory/window.json"));
    }

    #[test]
    fn a_drag_writes_once_it_settles_rather_than_every_frame() {
        // Sixty writes a second for a value the user is still choosing is a file the disk
        // does not need; one second after they let go survives a crash just as well.
        let path = scratch("settle");
        let mut state = WindowState { path: Some(path.clone()), ..WindowState::default() };
        assert!(state.observe((10.0, 10.0), &one_display(), 0), "the first observation is written");
        assert!(!state.observe((20.0, 10.0), &one_display(), 100), "mid-drag, not yet");
        assert!(!state.observe((30.0, 10.0), &one_display(), 900), "still settling");
        assert!(state.observe((40.0, 10.0), &one_display(), 1_100), "settled, so it is written");
        assert_eq!(Geometry::load(&path).position(), Some((40.0, 10.0)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_window_that_never_moved_is_not_written_again() {
        let path = scratch("still");
        let mut state = WindowState { path: Some(path.clone()), ..WindowState::default() };
        assert!(state.observe((10.0, 10.0), &one_display(), 0));
        assert!(!state.observe((10.0, 10.0), &one_display(), 5_000), "the same place is not news");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_session_that_ends_mid_drag_still_remembers_the_move() {
        let path = scratch("flush");
        let mut state = WindowState { path: Some(path.clone()), ..WindowState::default() };
        state.flush(&one_display(), 0);
        assert!(!path.exists(), "nothing observed, nothing written");

        state.observe((10.0, 10.0), &one_display(), 0);
        state.observe((50.0, 60.0), &one_display(), 100);
        state.flush(&one_display(), 200);
        assert_eq!(Geometry::load(&path).position(), Some((50.0, 60.0)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_state_with_no_path_never_writes_anything() {
        let mut state = WindowState::default();
        assert!(
            !state.observe((10.0, 10.0), &one_display(), 5_000),
            "nowhere to write is not a write"
        );
        state.flush(&one_display(), 9_000);
    }

    #[test]
    fn a_save_with_nothing_remembered_writes_nothing() {
        let path = scratch("empty");
        Geometry::default().save(&path);
        assert!(!path.exists(), "nothing to remember means no file at all");
    }

    // ── displays ─────────────────────────────────────────────────────────────────

    #[test]
    fn a_position_on_a_display_is_restored_as_it_stands() {
        // The one-display machine: a remembered corner is simply used, and nothing is refused.
        let (displays, primary, _) = measured();
        let placement =
            placement(Some((120.0, 64.0)), &displays, primary, (640.0, 400.0), Anchor::Center);
        assert_eq!(placement, Placement::Restored((120.0, 64.0)));
        assert_eq!(placement.position(), Some((120.0, 64.0)), "the remembered corner is applied");
    }

    #[test]
    fn a_position_from_a_display_that_is_gone_falls_back_to_the_anchor() {
        // The refusal, in numbers: `{"x":402.0,"y":11276.0}` was recorded while an external
        // monitor hung below the built-in one; that monitor is unplugged, and the coordinate is
        // now on no display at all. The panel must open at the anchor instead of being created
        // where the user cannot see it.
        let (displays, primary, remembered) = measured();
        let placement =
            placement(Some(remembered), &displays, primary, (640.0, 400.0), Anchor::Center);
        assert_eq!(
            placement,
            Placement::Replaced { rejected: (402.0, 11276.0), position: (1192.0, 782.0) },
            "the centre of the work area, and the refused corner named"
        );
    }

    #[test]
    fn a_refused_position_is_replaced_by_the_anchor_rather_than_clamped() {
        // Clamping the remembered y to the bottom edge (1964 - 400 = 1564) would leave a panel
        // half off the screen for no reason the user could see; the anchor is a place the design
        // chose, not a nudge.
        let (displays, primary, remembered) = measured();
        let placement =
            placement(Some(remembered), &displays, primary, (640.0, 400.0), Anchor::Center);
        let Some((_, y)) = placement.position() else {
            panic!("the anchor is available, so a position must be applied");
        };
        assert_ne!(y, 1564.0, "not clamped to the bottom edge");
        assert_eq!(y, 782.0, "centred in the work area");
    }

    #[test]
    fn a_display_with_a_negative_origin_is_a_display_like_any_other() {
        // A monitor placed to the left of or above the primary one reports negative coordinates.
        // That is a normal desk, not an off-screen position, and a naive `x >= 0` rule would
        // throw away every position on it.
        let above = ScreenRect::new(-1920.0, -1200.0, 1920.0, 1200.0);
        let displays = vec![display(), above];
        let placement =
            placement(Some((-900.0, -400.0)), &displays, Some(display()), (640.0, 400.0), Anchor::Center);
        assert_eq!(placement, Placement::Restored((-900.0, -400.0)));
        assert!(on_a_display((-900.0, -400.0), &displays));
    }

    #[test]
    fn a_corner_exactly_on_an_edge_is_on_that_display() {
        // The platform reports the window's own corner, and a corner exactly on a seam belongs to
        // both displays; refusing it would move a panel the user placed deliberately. The far
        // edge is as much a boundary as the near one.
        let left = ScreenRect::new(0.0, 0.0, 1920.0, 1080.0);
        let right = ScreenRect::new(1920.0, 0.0, 1280.0, 1024.0);
        let displays = vec![left, right];
        for corner in [(0.0, 0.0), (1920.0, 0.0), (0.0, 1080.0), (1920.0, 1024.0), (3200.0, 1024.0)] {
            assert!(on_a_display(corner, &displays), "{corner:?} is on a display");
        }
        assert!(!on_a_display((3200.5, 1024.0), &displays), "just past the right edge is off");
        assert!(!on_a_display((0.0, -0.5), &displays), "just above the top edge is off");
    }

    #[test]
    fn a_window_that_hangs_off_an_edge_is_still_restored() {
        // Only the corner is tested. A panel whose body overhangs the outer edge of a display is
        // honoured — the rule is "the corner is on no attached display", not "the window fits".
        let displays = one_display();
        let corner = (1900.0, 1060.0);
        assert_eq!(
            placement(Some(corner), &displays, Some(display()), (640.0, 400.0), Anchor::Center),
            Placement::Restored(corner)
        );
    }

    #[test]
    fn no_displays_reported_means_nothing_can_be_restored() {
        // The platform was asked and named no display. The remembered coordinate cannot be
        // verified, and there is no display to anchor on, so the platform's own placement — the
        // one placement that is on a display by construction — stands.
        let placement = placement(Some((402.0, 11276.0)), &[], None, (640.0, 400.0), Anchor::Center);
        assert_eq!(placement, Placement::Refused((402.0, 11276.0)));
        assert_eq!(placement.position(), None, "nothing is written over the platform's choice");

        assert!(!on_a_display((0.0, 0.0), &[]), "no display contains any point");
    }

    #[test]
    fn nothing_remembered_opens_at_the_configured_anchor() {
        // With no file (or no position in it) the anchor is the whole decision, which is what
        // makes `anchor` a setting rather than decoration.
        let (displays, primary, _) = measured();
        assert_eq!(
            placement(None, &displays, primary, (640.0, 400.0), Anchor::Center),
            Placement::Anchored((1192.0, 782.0))
        );
        assert_eq!(
            placement(None, &displays, primary, (640.0, 400.0), Anchor::TopLeft),
            Placement::Anchored((0.0, 25.0)),
            "the work area, not the raw frame: y=25 is below the menu bar"
        );
        assert_eq!(
            placement(None, &displays, primary, (640.0, 400.0), Anchor::TopCenter),
            Placement::Anchored((1192.0, 25.0))
        );
        assert_eq!(
            placement(None, &displays, primary, (640.0, 400.0), Anchor::TopRight),
            Placement::Anchored((2384.0, 25.0))
        );
    }

    #[test]
    fn nothing_remembered_and_no_display_leaves_the_platform_alone() {
        assert_eq!(placement(None, &[], None, (640.0, 400.0), Anchor::Center), Placement::Platform);
        assert_eq!(placement(None, &[], None, (640.0, 400.0), Anchor::Center).position(), None);
    }

    #[test]
    fn the_default_anchor_is_the_centre_of_the_display() {
        // The schema in `src/config.ts` and the patch row in `cordis.patch.yml` say `center`;
        // this is the half that would silently disagree if they drifted.
        assert_eq!(Anchor::default(), Anchor::Center);
        assert_eq!(Anchor::default().name(), "center");
    }

    #[test]
    fn the_anchor_vocabulary_round_trips_and_refuses_strangers() {
        for anchor in [Anchor::TopCenter, Anchor::TopLeft, Anchor::TopRight, Anchor::Center] {
            assert_eq!(Anchor::from_name(anchor.name()), Some(anchor), "{}", anchor.name());
        }
        assert_eq!(Anchor::from_name(" Center "), Some(Anchor::Center), "the env is trimmed");
        assert_eq!(Anchor::from_name("middle"), None, "an unknown name keeps the default");
    }

    #[test]
    fn an_off_display_position_is_never_written() {
        // The write path refuses what the read path would refuse: after the external monitor is
        // unplugged the window may sit at its remembered coordinate for a moment, and writing
        // that observation is how the bad value would become permanent.
        let path = scratch("off-display");
        let mut state = WindowState { path: Some(path.clone()), ..WindowState::default() };
        assert!(!state.observe((402.0, 11276.0), &one_display(), 0), "on no display, not written");
        assert!(!path.exists(), "and the file was not created to hold it");

        assert!(state.observe((402.0, 300.0), &one_display(), 2_000), "on the display, written");
        assert_eq!(Geometry::load(&path).position(), Some((402.0, 300.0)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_off_display_position_loaded_from_the_file_is_not_written_back_at_exit() {
        // The file is the other source of a remembered position. If startup refused it, the exit
        // flush must not rewrite it: otherwise the user is rescued on every start and the file
        // never stops carrying the coordinate.
        let path = scratch("flush-off-display");
        std::fs::write(&path, r#"{"x":402.0,"y":11276.0}"#).expect("write");
        let mut state = WindowState {
            geometry: Geometry::load(&path),
            path: Some(path.clone()),
            ..WindowState::default()
        };
        assert_eq!(state.position(), Some((402.0, 11276.0)), "the file still holds it");

        assert!(!state.flush(&one_display(), 1_000), "on no display, so not written");
        assert_eq!(
            Geometry::load(&path).position(),
            Some((402.0, 11276.0)),
            "the old file is untouched, and the next start refuses it in turn"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn no_reported_display_writes_nothing_at_all() {
        // A platform that will not enumerate displays must not leave the file holding a
        // coordinate nobody could verify.
        let path = scratch("flush-no-displays");
        let mut state = WindowState {
            geometry: Geometry { position: Some((10.0, 10.0)) },
            path: Some(path.clone()),
            ..WindowState::default()
        };
        assert!(!state.observe((10.0, 10.0), &[], 0), "unverifiable, so not remembered");
        assert!(!state.flush(&[], 5_000), "and not written at exit either");
        assert!(!path.exists());
    }

    #[test]
    fn the_marker_names_what_was_refused_and_what_was_done_instead() {
        // The exact sentences, not just their shape: this is the only record of the dropped
        // coordinate once the file has been rewritten, and "the panel came back somewhere odd"
        // has to be answerable from the marker alone.
        let (displays, primary, remembered) = measured();
        assert_eq!(
            placement(Some(remembered), &displays, primary, (640.0, 400.0), Anchor::Center)
                .note(Anchor::Center),
            Some(
                "window position rejected: 402,11276 is on no attached display; anchored at 1192,782 (center)"
                    .to_owned()
            ),
            "the refusal line"
        );
        assert_eq!(
            placement(Some((120.0, 64.0)), &displays, primary, (640.0, 400.0), Anchor::Center)
                .note(Anchor::Center),
            Some("window restored at 120,64".to_owned())
        );
        assert_eq!(
            placement(None, &displays, primary, (640.0, 400.0), Anchor::TopLeft)
                .note(Anchor::TopLeft),
            Some("window anchored at 0,25 (top-left)".to_owned())
        );
        assert_eq!(
            placement(Some(remembered), &[], None, (640.0, 400.0), Anchor::Center)
                .note(Anchor::Center),
            Some(
                "window position rejected: 402,11276 is on no attached display; no display to anchor on"
                    .to_owned()
            )
        );
        assert_eq!(
            Placement::Platform.note(Anchor::Center),
            None,
            "nothing extra to say: the pre-window line already said it"
        );
    }
}
