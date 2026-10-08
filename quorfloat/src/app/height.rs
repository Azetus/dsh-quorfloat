//! Animate the painted panel, resizing the native surface only to reserve or release room.
#[derive(Default)]
pub(super) struct Height {
    shown: Option<f32>,
    from: f32,
    target: f32,
    started: f64,
    pending: Option<eframe::egui::Vec2>,
}

impl Height {
    pub fn present(&mut self, now: f64, available: f32, reduce_motion: bool) -> f32 {
        let Some(shown) = self.shown else {
            self.shown = Some(available);
            self.from = available;
            self.target = available;
            return available;
        };
        let value = if self.target > available + 0.5 {
            // Wait for the OS resize acknowledgment before revealing more content.
            self.from = shown.min(available);
            self.started = now;
            self.from
        } else if reduce_motion {
            self.target
        } else {
            let t = ((now - self.started) / f64::from(crate::ui::theme::SPEED_EXPAND)).clamp(0.0, 1.0) as f32;
            self.from + (self.target - self.from) * (1.0 - (1.0 - t).powi(3))
        };
        self.shown = Some(value.min(available));
        self.shown.unwrap()
    }

    pub fn retarget(&mut self, target: f32, now: f64, pixels_per_point: f32) {
        let target = (target * pixels_per_point).ceil() / pixels_per_point;
        if (target - self.target).abs() < 0.5 / pixels_per_point { return; }
        self.from = self.shown.unwrap_or(target);
        self.target = target;
        self.started = now;
    }

    pub fn active(&self) -> bool {
        self.shown.is_some_and(|shown| (shown - self.target).abs() >= 0.1)
    }

    pub fn resize(&mut self, actual: eframe::egui::Vec2, width: f32, shadow: f32) -> Option<eframe::egui::Vec2> {
        let height = if self.active() { actual.y.max(self.target + shadow) } else { self.target + shadow };
        let wanted = eframe::egui::vec2(width, height);
        if self.pending.is_some_and(|pending| (pending - actual).length() < 0.5) { self.pending = None; }
        if self.pending == Some(wanted) { return None; }
        if self.pending.is_none() && (wanted - actual).length() < 0.5 { return None; }
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
}
