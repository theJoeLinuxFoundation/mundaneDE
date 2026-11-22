pub mod spring;

pub use spring::{SpringHandle, SpringId, SpringKind, SpringParams, MAX_DT};

use std::collections::HashMap;
use std::time::Duration;

/// Central animation engine. Owns all active springs and ticks them each frame.
pub struct AnimationEngine {
    springs: HashMap<SpringId, SpringHandle>,
    next_id: u64,
}

impl AnimationEngine {
    pub fn new() -> Self {
        Self {
            springs: HashMap::new(),
            next_id: 0,
        }
    }

    /// Allocate a new 2D position spring and return its ID. Equivalent to
    /// [`create_spring_kind`](Self::create_spring_kind) with `SpringKind::Position`.
    pub fn create_spring(&mut self, x: i32, y: i32, params: SpringParams) -> SpringId {
        self.create_spring_kind(x, y, params, SpringKind::Position)
    }

    /// Allocate a tagged spring. The wallpaper fade springs pass `SpringKind::Fade` so
    /// hot-reloading the `[spring]` TOML section doesn't accidentally retune them with
    /// the wrong params (their config lives under `[wallpaper].fade_*`).
    pub fn create_spring_kind(
        &mut self,
        x: i32,
        y: i32,
        params: SpringParams,
        kind: SpringKind,
    ) -> SpringId {
        let id = SpringId(self.next_id);
        self.next_id += 1;
        self.springs
            .insert(id, SpringHandle::with_kind(id, x, y, params, kind));
        id
    }

    /// Remove a spring (e.g. when a window is unmapped).
    pub fn remove_spring(&mut self, id: SpringId) {
        self.springs.remove(&id);
    }

    pub fn spring(&self, id: SpringId) -> Option<&SpringHandle> {
        self.springs.get(&id)
    }

    pub fn spring_mut(&mut self, id: SpringId) -> Option<&mut SpringHandle> {
        self.springs.get_mut(&id)
    }

    /// Iterate every spring mutably — used by the hot-reload path to re-apply tuning to
    /// existing springs.
    pub fn springs_mut(&mut self) -> impl Iterator<Item = &mut SpringHandle> {
        self.springs.values_mut()
    }

    pub fn settle_all(&mut self) {
        for spring in self.springs.values_mut() {
            spring.x.snap_to_target();
            spring.y.snap_to_target();
        }
    }

    /// Advance all springs by `dt`. Returns `true` if any spring is still animating.
    ///
    /// Each axis settles independently: once x is at rest we snap it to target and stop
    /// integrating it even if y is still moving. This saves CPU for the common
    /// edge-snap case (y settles fast, x ends up flush against a screen edge with no
    /// sub-pixel drift).
    pub fn tick(&mut self, dt: Duration) -> bool {
        let dt_secs = dt.min(MAX_DT).as_secs_f64();
        if dt_secs <= 0.0 {
            return self.has_active_animations();
        }

        let mut any_active = false;
        for spring in self.springs.values_mut() {
            if !spring.x.is_settled() {
                spring.x.tick(&spring.params, dt_secs);
                if spring.x.is_settled() {
                    spring.x.snap_to_target();
                } else {
                    any_active = true;
                }
            }
            if !spring.y.is_settled() {
                spring.y.tick(&spring.params, dt_secs);
                if spring.y.is_settled() {
                    spring.y.snap_to_target();
                } else {
                    any_active = true;
                }
            }
        }
        any_active
    }

    /// Returns `true` if any spring is still animating.
    pub fn has_active_animations(&self) -> bool {
        self.springs.values().any(|s| !s.is_settled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_create_and_remove() {
        let mut engine = AnimationEngine::new();
        let id = engine.create_spring(0, 0, SpringParams::default());
        assert!(engine.spring(id).is_some());
        engine.remove_spring(id);
        assert!(engine.spring(id).is_none());
    }

    #[test]
    fn reducing_motion_finishes_a_snap_and_crossfade_in_one_frame() {
        let mut engine = AnimationEngine::new();
        let ids = [SpringKind::Position, SpringKind::Fade].map(|kind| {
            let id = engine.create_spring_kind(0, 0, SpringParams::default(), kind);
            engine.spring_mut(id).unwrap().set_target(255, 100);
            id
        });
        engine.tick(Duration::from_millis(16));
        assert!(engine.has_active_animations());
        engine.settle_all();
        assert!(!engine.has_active_animations());
        for id in ids {
            assert_eq!(engine.spring(id).unwrap().rounded_position(), (255, 100));
        }
    }

    #[test]
    fn engine_tick_settles() {
        let mut engine = AnimationEngine::new();
        let id = engine.create_spring(0, 0, SpringParams::default());
        engine.spring_mut(id).unwrap().set_target(100, 100);

        assert!(engine.has_active_animations());

        // Tick for 10 seconds
        for _ in 0..600 {
            engine.tick(Duration::from_secs_f64(1.0 / 60.0));
        }

        assert!(!engine.has_active_animations());
        let (x, y) = engine.spring(id).unwrap().rounded_position();
        assert_eq!((x, y), (100, 100));
    }

    #[test]
    fn engine_snaps_settled_axis_to_exact_target() {
        let mut engine = AnimationEngine::new();
        let id = engine.create_spring(0, 0, SpringParams::default());
        engine.spring_mut(id).unwrap().set_target(100, 100);
        for _ in 0..600 {
            engine.tick(Duration::from_secs_f64(1.0 / 60.0));
        }
        let s = engine.spring(id).unwrap();
        // Settle must be exact — not "rounded == target". This is the no-drift
        // guarantee that prevents windows from sitting at e.g. (99.7, 100.3).
        assert_eq!(s.x.position, 100.0);
        assert_eq!(s.y.position, 100.0);
        assert_eq!(s.x.velocity, 0.0);
        assert_eq!(s.y.velocity, 0.0);
    }

    #[test]
    fn final_tick_reaches_the_exact_target_after_a_dropped_frame() {
        let mut engine = AnimationEngine::new();
        let id = engine.create_spring(0, 0, SpringParams::default());
        engine.spring_mut(id).unwrap().set_target(17, 93);
        for dt in [16, 180, 8, 33].into_iter().cycle().take(1000) {
            if !engine.has_active_animations() {
                break;
            }
            engine.tick(Duration::from_millis(dt));
            let s = engine.spring(id).unwrap();
            assert!((0.0..=17.0).contains(&s.x.position));
            assert!((0.0..=93.0).contains(&s.y.position));
        }
        let s = engine.spring(id).unwrap();
        assert_eq!((s.x.position, s.y.position), (17., 93.));
    }

    #[test]
    fn engine_clamps_large_dt() {
        let mut engine = AnimationEngine::new();
        let id = engine.create_spring(0, 0, SpringParams::default());
        engine.spring_mut(id).unwrap().set_target(100, 100);

        // A huge dt should not cause NaN or explosion
        engine.tick(Duration::from_secs(1));
        let (x, y) = engine.spring(id).unwrap().rounded_position();
        assert!(
            x.abs() < 10000 && y.abs() < 10000,
            "position exploded: ({x}, {y})"
        );
    }
}
