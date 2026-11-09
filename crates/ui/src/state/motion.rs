//! App-owned progress for toggle animations.
//!
//! GPUI's `with_animation` keeps progress in element state, which is discarded
//! when an element unmounts. The shell is unmounted under the quick-look
//! overlay, so toggles driven that way would replay from the opposite state
//! when it returns. Keeping the start time and endpoints in `App` makes the
//! rendered progress independent of mount lifetime.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::ui::theme::eased_progress;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ToggleMotion {
    epoch: u64,
    started_at: Instant,
    duration: Duration,
    from: f32,
    to: f32,
}

impl ToggleMotion {
    /// A settled value that renders immediately with no animation.
    pub(crate) fn settled(value: f32) -> Self {
        let value = value.clamp(0.0, 1.0);
        Self {
            epoch: 0,
            started_at: Instant::now(),
            duration: Duration::ZERO,
            from: value,
            to: value,
        }
    }

    /// Animates from the value currently on screen to `target`.
    pub(crate) fn animate_to(&mut self, target: f32, duration: Duration) {
        self.animate_to_at(target, duration, Instant::now());
    }

    /// Animates from the value at `now` to `target`, starting at `now`.
    pub(crate) fn animate_to_at(&mut self, target: f32, duration: Duration, now: Instant) {
        let target = target.clamp(0.0, 1.0);
        let from = self.value_at(now);
        if (from - target).abs() <= f32::EPSILON {
            self.epoch = self.epoch.wrapping_add(1);
            self.started_at = now;
            self.duration = Duration::ZERO;
            self.from = target;
            self.to = target;
            return;
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.started_at = now;
        self.duration = duration;
        self.from = from;
        self.to = target;
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) fn value(&self) -> f32 {
        self.value_at(Instant::now())
    }

    pub(crate) fn value_at(&self, now: Instant) -> f32 {
        let progress = eased_progress(self.started_at, self.duration, now);
        (self.from + (self.to - self.from) * progress).clamp(0.0, 1.0)
    }

    /// Duration for the frame-driver element. A settled motion returns zero so
    /// a remount paints once at the final value instead of replaying frames.
    pub(crate) fn driver_duration(&self) -> Duration {
        if self.is_active_at(Instant::now()) {
            self.duration
        } else {
            Duration::ZERO
        }
    }

    pub(crate) fn is_active_at(&self, now: Instant) -> bool {
        !self.duration.is_zero() && now.saturating_duration_since(self.started_at) < self.duration
    }

    pub(crate) fn duration(&self) -> Duration {
        self.duration
    }
}

/// One collapse animation per session. `0.0` is expanded and `1.0` is
/// collapsed; a retarget starts from the value currently on screen.
#[derive(Debug, Default)]
pub(crate) struct SessionCollapseMotions {
    motions: HashMap<i64, ToggleMotion>,
    epoch: u64,
}

impl SessionCollapseMotions {
    pub(crate) fn animate(
        &mut self,
        session_id: i64,
        collapsed: bool,
        duration: Duration,
        now: Instant,
    ) {
        let target = if collapsed { 1.0 } else { 0.0 };
        let motion = self
            .motions
            .entry(session_id)
            .or_insert_with(|| ToggleMotion::settled(1.0 - target));
        if !motion.is_active_at(now) {
            *motion = ToggleMotion::settled(1.0 - target);
        }
        motion.animate_to_at(target, duration, now);
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(crate) fn settle(&mut self, session_id: i64) {
        self.motions.remove(&session_id);
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Drops every finished motion and returns the active values.
    pub(crate) fn factors(&mut self, now: Instant) -> HashMap<i64, f32> {
        self.motions.retain(|_, motion| motion.is_active_at(now));
        self.motions
            .iter()
            .map(|(session_id, motion)| (*session_id, motion.value_at(now)))
            .collect()
    }

    pub(crate) fn max_duration(&self) -> Duration {
        self.motions
            .values()
            .map(ToggleMotion::duration)
            .max()
            .unwrap_or(Duration::ZERO)
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(crate) fn clear(&mut self) {
        self.motions.clear();
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.motions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_motion_renders_its_value_without_frames() {
        let motion = ToggleMotion::settled(1.0);

        assert_eq!(motion.value(), 1.0);
        assert_eq!(motion.driver_duration(), Duration::ZERO);
    }

    #[test]
    fn motion_interpolates_from_the_current_value_to_the_target() {
        let mut motion = ToggleMotion::settled(0.0);
        motion.animate_to(1.0, Duration::from_millis(100));
        let start = motion.started_at;
        let epoch = motion.epoch();

        assert_eq!(motion.value_at(start), 0.0);
        assert_eq!(motion.value_at(start + Duration::from_millis(100)), 1.0);
        let middle = motion.value_at(start + Duration::from_millis(50));
        assert!(middle > 0.0 && middle < 1.0);
        assert_eq!(motion.epoch(), epoch);
    }

    #[test]
    fn retargeting_starts_from_the_rendered_value() {
        let mut motion = ToggleMotion::settled(0.0);
        motion.animate_to(1.0, Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(20));

        motion.animate_to(0.0, Duration::from_millis(100));

        assert!(motion.from > 0.0 && motion.from < 1.0);
        assert_eq!(motion.to, 0.0);
        assert_eq!(motion.value_at(motion.started_at), motion.from);
    }

    #[test]
    fn collapse_motion_interpolates_and_stops_at_the_target() {
        let now = Instant::now();
        let mut motions = SessionCollapseMotions::default();
        motions.animate(7, true, Duration::from_millis(100), now);
        assert_eq!(motions.max_duration(), Duration::from_millis(100));

        let start = motions.factors(now);
        assert_eq!(start.get(&7).copied(), Some(0.0));

        let middle = motions.factors(now + Duration::from_millis(50));
        let factor = middle[&7];
        assert!(factor > 0.0 && factor < 1.0);

        let done = motions.factors(now + Duration::from_millis(100));
        assert!(done.is_empty());
        assert_eq!(motions.max_duration(), Duration::ZERO);
    }

    #[test]
    fn retargeting_a_collapse_motion_uses_the_current_value() {
        let now = Instant::now();
        let mut motions = SessionCollapseMotions::default();
        motions.animate(7, true, Duration::from_millis(100), now);
        let middle = motions.factors(now + Duration::from_millis(50))[&7];

        motions.animate(
            7,
            false,
            Duration::from_millis(100),
            now + Duration::from_millis(50),
        );
        let retargeted = motions.factors(now + Duration::from_millis(50))[&7];
        assert!((retargeted - middle).abs() < 0.001);
        assert_eq!(motions.factors(now + Duration::from_millis(150)).len(), 0);
    }

    #[test]
    fn settle_removes_only_the_requested_session() {
        let now = Instant::now();
        let mut motions = SessionCollapseMotions::default();
        motions.animate(7, true, Duration::from_millis(100), now);
        motions.animate(8, true, Duration::from_millis(100), now);
        motions.settle(7);

        let factors = motions.factors(now + Duration::from_millis(20));
        assert!(!factors.contains_key(&7));
        assert!(factors.contains_key(&8));
        assert!(!motions.is_empty());
        assert_eq!(motions.epoch(), 3);
        motions.clear();
        assert!(motions.is_empty());
    }
}
