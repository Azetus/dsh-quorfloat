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

    /// Write the remembered position atomically, if there is one.
    ///
    /// Failure is returned rather than swallowed: the window keeps its current position and
    /// the shell records a marker. A missing position writes nothing.
    ///
    /// @param path - where to write. Its directory is created when needed.
    /// @returns `Ok(())` when there is no position to write, or when the file now holds it;
    ///   an error with the path in its message otherwise.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let Some((x, y)) = self.position else {
            return Ok(());
        };
        let body = serde_json::json!({ "x": x, "y": y });
        crate::app::persist::write_atomic(path, body.to_string().as_bytes())
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

/// Whether a rectangle has at least the requested visible extent on one display.
///
/// The show guard uses this on the window's drag strip, not on the whole window: a panel with
/// only a few pixels of its top bar visible is technically on screen but cannot be grabbed, and
/// treating that as usable is how a window becomes stranded at a corner. Requiring a minimum
/// width and height keeps a partially off-screen window where the user put it, while bringing
/// back one whose handle is effectively gone.
///
/// The intersection is measured against one display at a time. A drag strip that straddles two
/// displays with a gap can be split into two smaller pieces; the conservative result is
/// "unusable" unless one piece alone is large enough.
///
/// An empty display list reports `false`; callers must not use that as a reason to move the
/// window, because there is nowhere to move it to.
///
/// @param rect - the rectangle to test, usually the top drag strip.
/// @param displays - every display the platform reports.
/// @param min_width - required visible width in the rectangle's units.
/// @param min_height - required visible height in the rectangle's units.
/// @returns whether one display contains at least that much of the rectangle.
#[must_use]
pub fn visible_at_least(
    rect: ScreenRect,
    displays: &[ScreenRect],
    min_width: f32,
    min_height: f32,
) -> bool {
    displays.iter().any(|display| {
        let left = rect.x.max(display.x);
        let right = (rect.x + rect.width).min(display.x + display.width);
        let top = rect.y.max(display.y);
        let bottom = (rect.y + rect.height).min(display.y + display.height);
        right - left >= min_width && bottom - top >= min_height
    })
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
    /// The most recent failed save, kept until it has been reported or a save succeeds.
    save_error: Option<String>,
    /// Whether the current error has already been handed to the shell for a marker line.
    ///
    /// A save is retried on later ticks when it failed; repeating the same marker every
    /// second would turn a transient disk problem into log spam.
    error_reported: bool,
}

impl WindowState {
    /// Read the remembered position, if there is one to read.
    #[must_use]
    pub fn load() -> Self {
        let path = path_from_env();
        let geometry = path.as_deref().map(Geometry::load).unwrap_or_default();
        Self {
            geometry,
            path,
            written: None,
            written_at: None,
            save_error: None,
            error_reported: false,
        }
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
    /// A failed save leaves `written` at its previous value so the next settled observation
    /// retries; `written_at` still moves forward so a failing disk does not turn every tick
    /// into a write attempt. The error is kept for [`Self::take_save_error`] and reported
    /// once per failure episode.
    ///
    /// @returns whether anything was written: "nowhere to write" is not a write, and a
    ///   caller that logs one would be logging a file that does not exist.
    fn write(&mut self, now: i64) -> bool {
        let Some(path) = &self.path else { return false };
        let position = self.geometry.position();
        match self.geometry.save(path) {
            Ok(()) => {
                self.written = position;
                self.written_at = Some(now);
                self.save_error = None;
                self.error_reported = false;
                true
            }
            Err(error) => {
                self.written_at = Some(now);
                if self.save_error.is_none() {
                    self.save_error = Some(error.to_string());
                }
                false
            }
        }
    }

    /// Take the most recent save error for a marker line.
    ///
    /// The error is returned once per failure episode. A later successful save resets the
    /// episode, so a new failure after recovery is reported again.
    ///
    /// @returns the error message, or `None` when there is nothing new to report.
    #[must_use]
    pub fn take_save_error(&mut self) -> Option<String> {
        if self.error_reported {
            return None;
        }
        let error = self.save_error.clone()?;
        self.error_reported = true;
        Some(error)
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
