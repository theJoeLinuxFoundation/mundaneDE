pub mod spring;

pub use spring::{SpringHandle, SpringId, SpringParams, MAX_DT};

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

    /// Allocate a new 2D position spring and return its ID.
    pub fn create_spring(&mut self, x: i32, y: i32, params: SpringParams) -> SpringId {
        let id = SpringId(self.next_id);
        self.next_id += 1;
        self.springs
            .insert(id, SpringHandle::new(id, x, y, params));
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

    /// Advance all springs by `dt`. Returns `true` if any spring is still animating.
    pub fn tick(&mut self, dt: Duration) -> bool {
        let dt_secs = dt.min(MAX_DT).as_secs_f64();
        if dt_secs <= 0.0 {
            return self.has_active_animations();
        }

        let mut any_active = false;
        for spring in self.springs.values_mut() {
            if !spring.is_settled() {
                spring.tick(dt_secs);
                if !spring.is_settled() {
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
    fn engine_clamps_large_dt() {
        let mut engine = AnimationEngine::new();
        let id = engine.create_spring(0, 0, SpringParams::default());
        engine.spring_mut(id).unwrap().set_target(100, 100);

        // A huge dt should not cause NaN or explosion
        engine.tick(Duration::from_secs(1));
        let (x, y) = engine.spring(id).unwrap().rounded_position();
        assert!(x.abs() < 10000 && y.abs() < 10000, "position exploded: ({x}, {y})");
    }
}
