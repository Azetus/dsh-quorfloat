//! Coordinates the native window height with the panel height the frontend reports.
//!
//! The visual transition is the frontend's job (CSS animates the panel's own
//! height); what lives in this process is the coordination, and it is load-bearing:
//! the window must make room *before* the panel
//! grows, a platform that never makes room must not keep the panel cropped or the
//! shell busy, and giving up is reported once.
//!
//! Rules:
//!
//! - a growth request waits for the platform to make room; the wait has a deadline
//!   (2 × the frontend's 220ms transition), and **a target that keeps growing does
//!   not extend it** — otherwise a streaming answer would grow the panel forever
//!   and the wait would never end;
//! - when the deadline passes, the panel caps itself at what the window actually
//!   gave, reported once (`height capped at …`); asking again would be a busy loop;
//! - the window later growing on its own re-arms the request, with a fresh
//!   deadline — a capped panel recovers when the platform lets it;
//! - shrink requests are sent immediately: shrinking never needs the platform's
//!   permission; identical requests are deduplicated.

/// Extra native space around the panel for its CSS shadow, logical pixels.
/// The design's shadow room: side 34, top 20, bottom 40 — the window is the panel
/// plus this.
pub const SHADOW_SIDE: f32 = 34.0;
pub const SHADOW_TOP: f32 = 20.0;
pub const SHADOW_BOTTOM: f32 = 40.0;

/// How long one growth request waits for the platform to make room: twice the
/// frontend's 220ms CSS transition.
pub const WAIT_FOR_NATIVE_ROOM_MS: i64 = 440;

/// Two native heights this close are the same height.
pub const ROOM_EPSILON: f32 = 0.5;

/// A panel shorter than this is a measurement error, not a request.
pub const MIN_PANEL_HEIGHT: f32 = 120.0;

/// What the shell should do about the window next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeightAction {
    /// Ask the platform for this native inner height.
    Resize { native: f32 },
    /// The platform never made room: present the panel at this height instead.
    Cap { available_panel: f32 },
}

/// The height coordination state machine.
///
/// Pure: time and the platform's actual height are parameters, so every rule is
/// testable without a window. The dispatcher owns one of these and calls it from
/// the frontend's height reports and its own periodic check.
#[derive(Debug, Clone)]
pub struct Height {
    /// Panel height the frontend last reported, in logical pixels.
    target: f32,
    /// Native inner height last asked of the platform.
    requested: Option<f32>,
    /// When the current wait for room began. Set by the request that began it and
    /// **not** reset by a target that keeps growing.
    waiting_since: Option<i64>,
    /// Whether the panel is capped to what the platform actually gave.
    capped: bool,
    /// Whether the cap has been reported since the last re-arm.
    cap_reported: bool,
}

impl Default for Height {
    fn default() -> Self {
        Self::new()
    }
}

impl Height {
    /// A machine with no target: nothing is asked for until the frontend reports.
    #[must_use]
    pub fn new() -> Self {
        Self { target: 0.0, requested: None, waiting_since: None, capped: false, cap_reported: false }
    }

    /// The panel height last reported.
    #[must_use]
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether the panel is currently capped to the available room.
    #[must_use]
    pub fn is_capped(&self) -> bool {
        self.capped
    }

    /// The native inner height that fits this panel height.
    #[must_use]
    pub fn native_for(panel: f32) -> f32 {
        panel + SHADOW_TOP + SHADOW_BOTTOM
    }

    /// Accept a new panel height from the frontend.
    ///
    /// @param panel - the panel height in logical pixels.
    /// @param now - caller-local time in milliseconds.
    /// @returns what to do with the window, if anything changed.
    pub fn retarget(&mut self, panel: f32, now: i64) -> Option<HeightAction> {
        let target = panel.max(MIN_PANEL_HEIGHT);
        self.target = target;
        let native = Self::native_for(target);
        if let Some(requested) = self.requested {
            if (requested - native).abs() <= ROOM_EPSILON && !self.capped {
                return None;
            }
        }
        self.requested = Some(native);
        // A fresh request starts a fresh wait; a target that keeps growing while a
        // wait is already running does not restart it (see the module rules).
        if self.waiting_since.is_none() {
            self.waiting_since = Some(now);
        }
        self.capped = false;
        self.cap_reported = false;
        Some(HeightAction::Resize { native })
    }

    /// Check the platform's answer to the outstanding request.
    ///
    /// Called periodically by the dispatcher, which owns the window and can read
    /// its real size.
    ///
    /// @param now - caller-local time in milliseconds.
    /// @param native - the window's actual inner height.
    /// @returns what to do next, if anything.
    pub fn tick(&mut self, now: i64, native: f32) -> Option<HeightAction> {
        let Some(requested) = self.requested else { return None };
        let satisfied = native >= requested - ROOM_EPSILON;

        if self.capped {
            // We accepted what the platform gave; only growth beyond it is news.
            if native > requested + ROOM_EPSILON {
                // The window grew on its own: ask for the real target again, with
                // a fresh deadline. This is the self-healing path.
                self.capped = false;
                self.cap_reported = false;
                let native = Self::native_for(self.target);
                self.requested = Some(native);
                self.waiting_since = Some(now);
                return Some(HeightAction::Resize { native });
            }
            return None;
        }

        if satisfied {
            self.waiting_since = None;
            return None;
        }

        // The platform has not made room yet.
        let since = self.waiting_since.unwrap_or(now);
        if now - since < WAIT_FOR_NATIVE_ROOM_MS {
            return None;
        }
        // Give up on this request: take what exists, say so once, and stop asking.
        self.capped = true;
        self.requested = Some(native);
        self.waiting_since = None;
        if self.cap_reported {
            return None;
        }
        self.cap_reported = true;
        Some(HeightAction::Cap {
            available_panel: (native - SHADOW_TOP - SHADOW_BOTTOM).max(MIN_PANEL_HEIGHT),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_asks_for_native_room_and_deduplicates_the_same_request() {
        let mut height = Height::new();
        assert_eq!(
            height.retarget(400.0, 0),
            Some(HeightAction::Resize { native: Height::native_for(400.0) }),
        );
        assert_eq!(height.retarget(400.0, 10), None, "the same request is not sent twice");
        assert_eq!(
            height.retarget(500.0, 20),
            Some(HeightAction::Resize { native: Height::native_for(500.0) }),
            "a new target is a new request",
        );
    }

    #[test]
    fn shrink_requests_are_sent_immediately_and_without_a_wait() {
        let mut height = Height::new();
        height.retarget(500.0, 0);
        assert_eq!(
            height.retarget(300.0, 100),
            Some(HeightAction::Resize { native: Height::native_for(300.0) }),
        );
        // The platform still reports the old, larger size: that satisfies the
        // shrink request, so nothing further is asked.
        assert_eq!(height.tick(120, Height::native_for(500.0)), None);
    }

    #[test]
    fn a_window_that_never_makes_room_is_given_up_on_after_the_deadline() {
        let mut height = Height::new();
        height.retarget(500.0, 0);
        assert_eq!(height.tick(100, 100.0), None, "within the deadline: keep waiting");
        let capped = height.tick(WAIT_FOR_NATIVE_ROOM_MS, 100.0);
        assert_eq!(
            capped,
            Some(HeightAction::Cap { available_panel: (100.0 - SHADOW_TOP - SHADOW_BOTTOM).max(MIN_PANEL_HEIGHT) }),
        );
        assert!(height.is_capped());
        assert_eq!(
            height.tick(WAIT_FOR_NATIVE_ROOM_MS + 50, 100.0),
            None,
            "the cap is reported once, not on every tick",
        );
    }

    #[test]
    fn a_growing_target_does_not_extend_the_wait() {
        let mut height = Height::new();
        height.retarget(400.0, 0);
        // The stream keeps growing the target while the platform does nothing:
        // the deadline stays anchored to the first request.
        height.retarget(450.0, 100);
        height.retarget(500.0, 200);
        assert_eq!(height.tick(300, 100.0), None, "still within the first wait");
        assert!(
            matches!(height.tick(WAIT_FOR_NATIVE_ROOM_MS, 100.0), Some(HeightAction::Cap { .. })),
            "the first deadline ends the wait",
        );
    }

    #[test]
    fn a_window_that_later_grows_re_arms_with_a_fresh_deadline() {
        let mut height = Height::new();
        height.retarget(500.0, 0);
        assert!(matches!(height.tick(WAIT_FOR_NATIVE_ROOM_MS, 100.0), Some(HeightAction::Cap { .. })));

        // The platform grows on its own, past what we accepted.
        assert_eq!(
            height.tick(1_000, 300.0),
            Some(HeightAction::Resize { native: Height::native_for(500.0) }),
            "self-healing: ask for the real target again",
        );
        assert!(!height.is_capped());
        // …and the new request gets its own deadline, measured from the re-arm.
        assert_eq!(height.tick(1_200, 300.0), None);
        assert!(
            matches!(height.tick(1_000 + WAIT_FOR_NATIVE_ROOM_MS, 300.0), Some(HeightAction::Cap { .. })),
        );
    }

    #[test]
    fn a_target_below_the_floor_is_a_measurement_error_not_a_request() {
        let mut height = Height::new();
        assert_eq!(
            height.retarget(3.0, 0),
            Some(HeightAction::Resize { native: Height::native_for(MIN_PANEL_HEIGHT) }),
        );
        assert_eq!(height.target(), MIN_PANEL_HEIGHT);
    }

    #[test]
    fn nothing_is_asked_before_the_frontend_reports() {
        let mut height = Height::new();
        assert_eq!(height.tick(0, 400.0), None);
        assert_eq!(height.target(), 0.0);
    }
}
