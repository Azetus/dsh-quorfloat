//! Animate the painted panel, resizing the native surface only to reserve or release room.

/// How long the panel waits for the platform to confirm a resize before giving up on it.
///
/// The wait is what keeps a growing panel from painting past the window that holds it, and the
/// normal case is a frame or two. Past this it buys nothing: the panel is held at its old height
/// for the whole wait, so a platform that never honors the request — a window manager clamping the
/// frame to the screen, a command that went nowhere — would leave it there, and, because a wait
/// counts as motion, would have the render loop repainting at full rate for as long as the panel
/// is on screen. Two animations' worth of patience is generous for an acknowledgment, and short
/// enough that nobody sees a stalled panel.
const WAIT_FOR_NATIVE_ROOM: f64 = 2.0 * crate::ui::theme::SPEED_EXPAND as f64;

/// How much of a height difference counts as a different height, in logical pixels.
const ROOM_EPSILON: f32 = 0.5;

#[derive(Default)]
pub(super) struct Height {
    shown: Option<f32>,
    from: f32,
    target: f32,
    /// Where the panel is heading: [`Self::target`], or the room the window actually offers once a
    /// wait has been given up on. [`Self::active`] compares against this, so a panel that has
    /// stopped moving also stops asking for another frame.
    aimed: f32,
    started: f64,
    /// When the current wait for native room began, if one is in progress.
    waiting_since: Option<f64>,
    /// Whether the room [`Self::target`] needs has already been given up on.
    capped: bool,
    /// The target that could not be reached, until the caller has been told about it.
    unreachable: Option<f32>,
    pending: Option<eframe::egui::Vec2>,
}

impl Height {
    /// How tall the panel should be painted this pass.
    ///
    /// @param now - seconds on the monotonic clock the animation is measured against.
    /// @param available - how tall a panel the window can actually show: its inner height less the
    ///   room the shadow needs.
    /// @param reduce_motion - jump to the target instead of easing toward it.
    /// @returns the height to paint the panel at; never more than `available`.
    pub fn present(&mut self, now: f64, available: f32, reduce_motion: bool) -> f32 {
        let Some(shown) = self.shown else {
            self.shown = Some(available);
            self.from = available;
            self.target = available;
            self.aimed = available;
            return available;
        };
        let aim = self.aim(now, available);
        if (aim - self.aimed).abs() > ROOM_EPSILON {
            // A new destination — a retarget, room appearing after a wait, or a window that shrank
            // under the panel. Start from where the panel is, so each of them is a transition
            // rather than a jump.
            self.from = shown.min(available);
            self.started = now;
        }
        self.aimed = aim;
        let value = if aim > available + ROOM_EPSILON {
            // Grow only once the platform has actually made room: a panel taller than the window
            // that holds it has its footer clipped off the bottom.
            self.from = shown.min(available);
            self.started = now;
            self.from
        } else if reduce_motion {
            aim
        } else {
            let t = ((now - self.started) / f64::from(crate::ui::theme::SPEED_EXPAND)).clamp(0.0, 1.0) as f32;
            self.from + (aim - self.from) * (1.0 - (1.0 - t).powi(3))
        };
        self.shown = Some(value.min(available));
        self.shown.unwrap()
    }

    /// Where the panel should be going: what the content asked for, or — when the platform has not
    /// made room for it within [`WAIT_FOR_NATIVE_ROOM`] — the room the window actually offers.
    ///
    /// The second answer is the difference between "waiting" and "stuck": the panel scrolls its
    /// content inside the room it has, instead of holding a height it can never reach.
    ///
    /// @param now - seconds on the monotonic clock.
    /// @param available - the room the window offers.
    /// @returns the height to head for.
    fn aim(&mut self, now: f64, available: f32) -> f32 {
        if self.target <= available + ROOM_EPSILON {
            // The room is there, or the content shrank back inside it: nothing to wait for.
            self.waiting_since = None;
            self.capped = false;
            self.unreachable = None;
            return self.target;
        }
        // A target that keeps growing does not extend the wait: `waiting_since` belongs to the
        // episode, not to the height, or a stream of tokens would keep the panel held forever.
        let since = *self.waiting_since.get_or_insert(now);
        if now - since < WAIT_FOR_NATIVE_ROOM {
            return self.target;
        }
        if !self.capped {
            self.capped = true;
            self.unreachable = Some(self.target);
        }
        self.target.min(available)
    }

    pub fn retarget(&mut self, target: f32, now: f64, pixels_per_point: f32) {
        let target = (target * pixels_per_point).ceil() / pixels_per_point;
        if (target - self.target).abs() < ROOM_EPSILON / pixels_per_point { return; }
        self.from = self.shown.unwrap_or(target);
        self.target = target;
        // Heading for the new target until `present` decides otherwise: `active` has to be able to
        // say "this panel is about to move", or the frame that would start the move is never asked for.
        self.aimed = target;
        self.started = now;
    }

    pub fn active(&self) -> bool {
        self.shown.is_some_and(|shown| (shown - self.aimed).abs() >= 0.1)
    }

    /// Take the target the window never made room for, once the wait for it has been given up on.
    ///
    /// Only meaningful straight after [`Self::present`], which is where the decision is made. The
    /// panel carries on at the room it has either way; this is for putting the give-up on the
    /// record, once per episode.
    ///
    /// @returns the height that could not be reached, or `None` when there is nothing to report.
    #[must_use]
    pub fn unreachable_target(&mut self) -> Option<f32> {
        self.unreachable.take()
    }

    pub fn resize(&mut self, actual: eframe::egui::Vec2, width: f32, shadow: f32) -> Option<eframe::egui::Vec2> {
        // While the panel is moving, reserve the room the move is heading for, so growth has
        // somewhere to go the moment it is painted. A settled panel asks for the height its content
        // wants — after a give-up too, deliberately: a platform that refused the size once may grant
        // it later, and an unchanged request is deduplicated below rather than repeated every frame.
        let height = if self.active() { actual.y.max(self.aimed + shadow) } else { self.target + shadow };
        let wanted = eframe::egui::vec2(width, height);
        if self.pending.is_some_and(|pending| (pending - actual).length() < ROOM_EPSILON) { self.pending = None; }
        if self.pending == Some(wanted) { return None; }
        if self.pending.is_none() && (wanted - actual).length() < ROOM_EPSILON { return None; }
        self.pending = Some(wanted);
        Some(wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;
    #[test]
    fn growth_waits_for_native_room_and_shrink_releases_it_only_after_animation() {
        let mut h = Height::default();
        h.present(0.0, 200.0, false);
        h.retarget(560.0, 0.0, 2.0);
        assert_eq!(h.resize(vec2(708.0, 260.0), 708.0, 60.0), Some(vec2(708.0, 620.0)));
        assert_eq!(h.present(0.05, 200.0, false), 200.0);
        assert_eq!(h.resize(vec2(708.0, 260.0), 708.0, 60.0), None, "deduplicate in-flight commands");
        let mid = h.present(0.1, 560.0, false);
        assert!(mid > 200.0 && mid < 560.0);
        assert_eq!(h.resize(vec2(708.0, 620.0), 708.0, 60.0), None);
        h.retarget(300.0, 0.1, 2.0);
        assert_eq!(h.present(0.1, 560.0, false), mid, "reversal is continuous");
        assert_eq!(h.resize(vec2(708.0, 620.0), 708.0, 60.0), None);
        assert_eq!(h.present(0.5, 560.0, false), 300.0);
        assert_eq!(h.resize(vec2(708.0, 620.0), 708.0, 60.0), Some(vec2(708.0, 360.0)));
        h.retarget(400.0, 0.6, 2.0);
        assert_eq!(h.resize(vec2(708.0, 620.0), 708.0, 60.0), Some(vec2(708.0, 620.0)), "cancel pending shrink on reversal");
        assert_eq!(h.present(0.6, 560.0, true), 400.0);
    }

    #[test]
    fn a_window_that_never_makes_room_is_given_up_on_rather_than_held_forever() {
        use crate::ui::theme::SPEED_EXPAND;
        let deadline = 2.0 * f64::from(SPEED_EXPAND);
        let mut h = Height::default();
        h.present(0.0, 200.0, false);
        h.retarget(560.0, 0.0, 2.0);
        assert_eq!(h.resize(vec2(708.0, 260.0), 708.0, 60.0), Some(vec2(708.0, 620.0)), "the room is asked for");
        // Inside the deadline the panel is held at what the window has and the caller keeps
        // painting: the resize may still arrive.
        assert_eq!(h.present(0.1, 200.0, false), 200.0);
        assert!(h.active(), "waiting for room is work in progress");
        // Past it, the wait is over even though the room never came: the panel aims at the room the
        // window actually offers, so the caller stops repainting and the content scrolls instead of
        // a panel taller than its window having its footer clipped off.
        assert_eq!(h.present(0.11 + deadline, 200.0, false), 200.0);
        assert!(!h.active(), "a window that cannot grow must not keep the render loop awake");
        assert_eq!(h.resize(vec2(708.0, 260.0), 708.0, 60.0), None, "the request in flight is not repeated");
        assert_eq!(h.unreachable_target(), Some(560.0), "the give-up is reported once");
        assert_eq!(h.unreachable_target(), None);
        // With reduce-motion on there is no easing to finish, and the give-up still holds.
        assert_eq!(h.present(0.6, 200.0, true), 200.0);
        assert!(!h.active());
        // Content that keeps arriving while the window is stuck does not restart the wait.
        h.retarget(700.0, 1.0, 2.0);
        assert_eq!(h.present(2.0, 200.0, false), 200.0);
        assert!(!h.active());
        assert_eq!(h.unreachable_target(), None, "and a panel that stays capped does not repeat itself");
        // Room that arrives later is used, and the panel moves into it rather than jumping.
        assert_eq!(h.present(3.0, 700.0, false), 200.0, "the room arriving is not itself a jump");
        let grown = h.present(3.05, 700.0, false);
        assert!(grown > 200.0 && grown < 700.0, "recovery animates: {grown}");
        assert_eq!(h.present(4.0, 700.0, false), 700.0);
        assert!(!h.active());
        // A later growth gets its own deadline instead of inheriting the old one.
        h.retarget(900.0, 4.0, 2.0);
        assert_eq!(h.present(4.1, 700.0, false), 700.0);
        assert!(h.active(), "a new wait is work in progress");
        assert_eq!(h.present(4.11 + deadline, 700.0, false), 700.0);
        assert!(!h.active());
        assert_eq!(h.unreachable_target(), Some(900.0), "a new episode is reported in its own right");
    }
}
