//! Where the panel was last time, so it comes back there.
//!
//! Window geometry is presentation state, which the design assigns to this process rather
//! than to the host (`docs/dsh-quorfloat.md`): the panel is a thing the user moves around,
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
//! - **Garbage is refused, not clamped.** A position that is not on any screen is worse
//!   than no position: the hotkey would reveal a panel the user cannot see.

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
    /// @param position - the window's current top-left corner.
    /// @param now - caller-local time in milliseconds.
    /// @returns whether a write happened, for the caller's log line.
    pub fn observe(&mut self, position: (f32, f32), now: i64) -> bool {
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
    /// @param now - caller-local time in milliseconds.
    pub fn flush(&mut self, now: i64) -> bool {
        if self.geometry.position().is_none() || self.written == self.geometry.position() {
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

/// Whether a position is one a window could actually be at.
///
/// Deliberately loose: negative coordinates are normal on a multi-monitor desktop, and this
/// process cannot enumerate the screens from here. What it rejects is the shapes that come
/// from a truncated write or a hand-edited file — NaN, infinities, and coordinates so large
/// that no desktop has them — because restoring one of those hides the panel off-screen,
/// and a panel a hotkey reveals nowhere is worse than a panel in the corner.
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
        assert!(state.observe((10.0, 10.0), 0), "the first observation is written");
        assert!(!state.observe((20.0, 10.0), 100), "mid-drag, not yet");
        assert!(!state.observe((30.0, 10.0), 900), "still settling");
        assert!(state.observe((40.0, 10.0), 1_100), "settled, so it is written");
        assert_eq!(Geometry::load(&path).position(), Some((40.0, 10.0)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_window_that_never_moved_is_not_written_again() {
        let path = scratch("still");
        let mut state = WindowState { path: Some(path.clone()), ..WindowState::default() };
        assert!(state.observe((10.0, 10.0), 0));
        assert!(!state.observe((10.0, 10.0), 5_000), "the same place is not news");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_session_that_ends_mid_drag_still_remembers_the_move() {
        let path = scratch("flush");
        let mut state = WindowState { path: Some(path.clone()), ..WindowState::default() };
        state.flush(0);
        assert!(!path.exists(), "nothing observed, nothing written");

        state.observe((10.0, 10.0), 0);
        state.observe((50.0, 60.0), 100);
        state.flush(200);
        assert_eq!(Geometry::load(&path).position(), Some((50.0, 60.0)));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_state_with_no_path_never_writes_anything() {
        let mut state = WindowState::default();
        assert!(!state.observe((10.0, 10.0), 5_000), "nowhere to write is not a write");
        state.flush(9_000);
    }

    #[test]
    fn a_save_with_nothing_remembered_writes_nothing() {
        let path = scratch("empty");
        Geometry::default().save(&path);
        assert!(!path.exists(), "nothing to remember means no file at all");
    }
}
