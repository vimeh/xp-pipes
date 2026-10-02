//! Render eligibility (0100dcd5) and the process-global fade clock (0100ecfd).
//! Time is supplied, never read or slept here. Source timestamps are stored f32;
//! comparisons and double constants are not prematurely rounded to f32.

pub struct RenderGate {
    speed: u8,
    last: f32,
}
impl RenderGate {
    pub fn new(speed: u8) -> Self {
        assert!(speed <= 20);
        Self { speed, last: 0.0 }
    }

    pub fn accepts(&mut self, now: f64) -> bool {
        if self.speed == 20 {
            return true;
        }
        if now - f64::from(self.last) > f64::from(20 - self.speed) * 0.005 {
            self.last = stored_time(now);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeStep {
    Wait,
    Overlay,
    Clear,
}

#[derive(Default)]
pub struct FadeClock {
    last: Option<f32>,
    remaining: u8,
}
impl FadeClock {
    pub fn step(&mut self, now: f64) -> FadeStep {
        let previous = *self.last.get_or_insert_with(|| stored_time(now));
        if self.remaining == 0 {
            self.remaining = 30;
        }
        if now - f64::from(previous) > 0.016 {
            self.remaining -= 1;
            self.last = Some(stored_time(now));
            if self.remaining == 0 {
                FadeStep::Clear
            } else {
                FadeStep::Overlay
            }
        } else {
            FadeStep::Wait
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "reference explicitly stores its timer in a single-precision field"
)]
fn stored_time(now: f64) -> f32 {
    now as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_speed_uses_a_strict_gate_without_backlog_replay() {
        for speed in 0..20 {
            let mut gate = RenderGate::new(speed);
            let threshold = f64::from(20 - speed) * 0.005;
            assert!(!gate.accepts(0.0));
            assert!(!gate.accepts(threshold));
            assert!(gate.accepts(threshold + 1e-8));
            assert!(gate.accepts(1000.0));
            assert!(!gate.accepts(1000.0));
            assert!(!gate.accepts(1000.0 + threshold / 2.0));
        }
        let mut gate = RenderGate::new(20);
        assert!(gate.accepts(0.0));
        assert!(gate.accepts(0.0));
        assert!(gate.accepts(1000.0));
        assert_eq!(
            gate.last.to_bits(),
            0,
            "speed 20 bypasses even the timestamp store"
        );
    }
    #[test]
    fn shared_countdown_draws_twenty_nine_overlays_then_clears() {
        let mut clock = FadeClock::default();
        assert_eq!(clock.step(0.0), FadeStep::Wait);
        assert_eq!(clock.step(0.016), FadeStep::Wait);
        for count in 1..=29 {
            let now = f64::from(count) * 0.020;
            assert_eq!(clock.step(now), FadeStep::Overlay);
            // A second output uses the SAME process clock, not its own 30 steps.
            assert_eq!(clock.step(now), FadeStep::Wait);
        }
        assert_eq!(clock.step(0.600), FadeStep::Clear);
        assert_eq!(clock.step(0.600), FadeStep::Wait);
        assert_eq!(
            clock.step(100.0),
            FadeStep::Overlay,
            "long pauses do not fast-forward"
        );
        assert_eq!(clock.remaining, 29);
    }
    #[test]
    fn repaint_does_not_reinitialize_shared_fade_state() {
        let mut clock = FadeClock::default();
        clock.step(0.0);
        clock.step(0.020);
        // No fade call during another scene or a resource rebuild.
        assert_eq!(clock.step(1.0), FadeStep::Overlay);
        assert_eq!(clock.remaining, 28);
    }
}
