//! The design's visibility transition, independent of native window scheduling.
//! Retargeting samples the current value, so a hotkey pressed mid-flight never jumps.

use eframe::egui;
use crate::ui::theme;

#[derive(Debug, Default)]
pub(super) struct Appearance {
    from: f32,
    to: f32,
    started: f64,
    duration: f64,
}

impl Appearance {
    /// The opacity/progress at this monotonic UI time.
    pub(super) fn value(&self, now: f64) -> f32 {
        if self.duration == 0.0 { return self.to; }
        let elapsed = ((now - self.started) / self.duration).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * css_ease(elapsed) as f32
    }

    pub(super) fn active(&self, now: f64) -> bool {
        self.from != self.to && now < self.started + self.duration
    }

    /// Repeated commands do not restart a transition; reduced motion settles immediately.
    pub(super) fn retarget(&mut self, visible: bool, now: f64, reduced: bool) {
        let to = if visible { 1.0 } else { 0.0 };
        if reduced {
            self.from = to; self.to = to; self.duration = 0.0;
        } else if self.to != to {
            self.from = self.value(now);
            self.to = to;
            self.started = now;
            self.duration = f64::from(theme::SPEED_VISIBILITY * (to - self.from).abs());
        }
    }

    /// CSS transform-origin: 50% 0; translateY(-6px) scale(.985).
    pub(super) fn transform(value: f32, width: f32) -> egui::emath::TSTransform {
        let away = 1.0 - value;
        let scale = 1.0 - (1.0 - theme::AWAY_SCALE) * away;
        let origin = egui::vec2(width / 2.0, f32::from(theme::SHADOW_ROOM_TOP));
        egui::emath::TSTransform::new(origin * (1.0 - scale) + egui::vec2(0.0, theme::AWAY_Y * away), scale)
    }
}

/// CSS's default `ease`: cubic-bezier(.25, .1, .25, 1). Invert x before sampling y.
fn css_ease(x: f64) -> f64 {
    if x <= 0.0 || x >= 1.0 { return x.clamp(0.0, 1.0); }
    let bezier = |t: f64, a: f64, b: f64| 3.0 * (1.0 - t).powi(2) * t * a + 3.0 * (1.0 - t) * t * t * b + t.powi(3);
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        let t = (low + high) / 2.0;
        if bezier(t, 0.25, 0.25) < x { low = t; } else { high = t; }
    }
    bezier((low + high) / 2.0, 0.1, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_design_curve_and_top_center_transform_match_css() {
        assert!((css_ease(0.5) - 0.802403).abs() < 0.00001);
        let away = Appearance::transform(0.0, 708.0);
        assert!((away.scaling - 0.985).abs() < 0.00001);
        assert_eq!(away * egui::pos2(354.0, 20.0), egui::pos2(354.0, 14.0));
        assert_eq!(Appearance::transform(1.0, 708.0), egui::emath::TSTransform::IDENTITY);
    }

    #[test]
    fn reversal_is_continuous_and_repeated_commands_do_not_restart_it() {
        let mut motion = Appearance::default();
        motion.retarget(true, 1.0, false);
        let halfway = motion.value(1.09);
        assert!((halfway - 0.802403).abs() < 0.00001);
        motion.retarget(true, 1.09, false);
        assert_eq!(motion.value(1.09), halfway);
        motion.retarget(false, 1.09, false);
        assert_eq!(motion.value(1.09), halfway);
        assert!(motion.value(1.13) < halfway);
        motion.retarget(true, 1.13, false);
        assert!(motion.active(1.13));
        assert_eq!(motion.value(1.4), 1.0);
        assert!(!motion.active(1.4));
        motion.retarget(false, 1.4, true);
        assert_eq!(motion.value(1.4), 0.0);
        assert!(!motion.active(1.4));
    }
}
