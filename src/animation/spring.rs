//! Single spring physics engine for mundaneDE.
//!
//! Every animated value in the compositor (window position, opacity, scale) is driven
//! by a damped harmonic oscillator. The governing equation is:
//!
//! ```text
//! a = (-stiffness * displacement - damping * velocity) / mass
//! ```
//!
//! Integration uses the semi-implicit Euler method (symplectic Euler):
//! ```text
//! velocity += acceleration * dt
//! position += velocity * dt
//! ```
//!
//! This method is unconditionally stable for spring-damper systems at typical frame
//! rates (60 Hz) and conserves energy better than explicit Euler.
//!
//! ## Tuning advice
//!
//! The **damping ratio** determines the character of the animation:
//!
//! ```text
//! zeta = damping / (2 * sqrt(stiffness * mass))
//! ```
//!
//! - `zeta < 1.0` — underdamped: springy overshoot (the default at zeta=0.5)
//! - `zeta = 1.0` — critically damped: fastest convergence without overshoot
//! - `zeta > 1.0` — overdamped: sluggish return, no overshoot
//!
//! Default parameters (stiffness=100, damping=10, mass=1) give zeta=0.5,
//! a pleasantly springy feel with mild overshoot.

use std::time::Duration;

/// Parameters that define a spring's physical properties.
#[derive(Debug, Clone, Copy)]
pub struct SpringParams {
    /// Stiffness constant (k). Higher values = stiffer, faster oscillation.
    pub stiffness: f64,
    /// Damping coefficient (d). Higher values = more friction, less bounce.
    pub damping: f64,
    /// Mass (m). Higher values = more inertia, slower acceleration.
    pub mass: f64,
}

impl Default for SpringParams {
    fn default() -> Self {
        Self {
            stiffness: 100.0,
            damping: 10.0,
            mass: 1.0,
        }
    }
}

/// The instantaneous state of a single 1D spring.
#[derive(Debug, Clone, Copy)]
pub struct SpringState {
    pub position: f64,
    pub velocity: f64,
    pub target: f64,
}

impl SpringState {
    pub fn new(initial: f64) -> Self {
        Self {
            position: initial,
            velocity: 0.0,
            target: initial,
        }
    }

    /// Returns `true` if the spring is at rest (within epsilon of target with near-zero velocity).
    pub fn is_settled(&self, epsilon: f64) -> bool {
        (self.position - self.target).abs() < epsilon && self.velocity.abs() < epsilon
    }

    /// Advance the spring by `dt` seconds using semi-implicit Euler integration.
    pub fn tick(&mut self, params: &SpringParams, dt: f64) {
        let displacement = self.position - self.target;
        let acceleration =
            (-params.stiffness * displacement - params.damping * self.velocity) / params.mass;
        self.velocity += acceleration * dt;
        self.position += self.velocity * dt;
    }
}

/// Unique identifier for a spring managed by the [`AnimationEngine`](super::AnimationEngine).
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct SpringId(pub(super) u64);

/// A 2D spring-driven animated position.
///
/// Wraps two independent [`SpringState`] values (x and y) with shared [`SpringParams`].
/// Used to animate window positions through the magnetic window manager.
pub struct SpringHandle {
    /// Identity of this spring within the [`AnimationEngine`](super::AnimationEngine);
    /// the engine keys on it, so the handle itself doesn't read it back.
    #[allow(dead_code)]
    pub id: SpringId,
    pub x: SpringState,
    pub y: SpringState,
    pub params: SpringParams,
}

impl SpringHandle {
    pub fn new(id: SpringId, initial_x: i32, initial_y: i32, params: SpringParams) -> Self {
        Self {
            id,
            x: SpringState::new(initial_x as f64),
            y: SpringState::new(initial_y as f64),
            params,
        }
    }

    /// Set the target position that the spring will animate toward.
    pub fn set_target(&mut self, x: i32, y: i32) {
        self.x.target = x as f64;
        self.y.target = y as f64;
    }

    /// Get the current position rounded to integer pixels.
    pub fn rounded_position(&self) -> (i32, i32) {
        (self.x.position.round() as i32, self.y.position.round() as i32)
    }

    /// Returns `true` if both axes have settled.
    pub fn is_settled(&self) -> bool {
        self.x.is_settled(SETTLE_EPSILON) && self.y.is_settled(SETTLE_EPSILON)
    }

    /// Advance both axes by `dt` seconds.
    pub fn tick(&mut self, dt: f64) {
        self.x.tick(&self.params, dt);
        self.y.tick(&self.params, dt);
    }
}

/// Settlement threshold: half a pixel.
const SETTLE_EPSILON: f64 = 0.5;

/// Maximum dt to prevent instability on dropped frames (clamped to ~2 frames at 60 Hz).
pub const MAX_DT: Duration = Duration::from_millis(33);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_converges_to_target() {
        let params = SpringParams::default();
        let mut state = SpringState::new(0.0);
        state.target = 100.0;

        // Simulate 5 seconds at 60 fps
        for _ in 0..300 {
            state.tick(&params, 1.0 / 60.0);
        }

        assert!(
            (state.position - 100.0).abs() < 0.01,
            "spring should converge to target, got {}",
            state.position
        );
        assert!(
            state.velocity.abs() < 0.01,
            "velocity should be near zero, got {}",
            state.velocity
        );
    }

    #[test]
    fn spring_settles() {
        let params = SpringParams::default();
        let mut state = SpringState::new(0.0);
        state.target = 50.0;

        for _ in 0..600 {
            state.tick(&params, 1.0 / 60.0);
        }

        assert!(state.is_settled(SETTLE_EPSILON));
    }

    #[test]
    fn spring_handle_2d() {
        let params = SpringParams::default();
        let mut handle = SpringHandle::new(SpringId(0), 0, 0, params);
        handle.set_target(100, 200);

        for _ in 0..600 {
            handle.tick(1.0 / 60.0);
        }

        let (x, y) = handle.rounded_position();
        assert_eq!(x, 100);
        assert_eq!(y, 200);
        assert!(handle.is_settled());
    }

    #[test]
    fn spring_at_target_is_settled() {
        let handle = SpringHandle::new(SpringId(0), 50, 50, SpringParams::default());
        assert!(handle.is_settled());
    }

    #[test]
    fn underdamped_spring_overshoots() {
        let params = SpringParams {
            stiffness: 100.0,
            damping: 5.0, // zeta = 0.25, quite bouncy
            mass: 1.0,
        };
        let mut state = SpringState::new(0.0);
        state.target = 100.0;

        let mut max_pos = 0.0f64;
        for _ in 0..300 {
            state.tick(&params, 1.0 / 60.0);
            max_pos = max_pos.max(state.position);
        }

        assert!(
            max_pos > 100.0,
            "underdamped spring should overshoot, max was {}",
            max_pos
        );
    }
}
