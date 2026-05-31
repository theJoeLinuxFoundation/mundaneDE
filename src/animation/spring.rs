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
//! rates (60 Hz) and conserves energy better than explicit Euler. For stiffer springs
//! (or larger `dt` after a dropped frame) we sub-step internally so a single `tick(dt)`
//! call never integrates further than [`MAX_STEP`] in one go.
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

impl SpringParams {
    /// Build params from a target damping ratio. ζ = damping / (2·√(k·m)), so
    /// `damping = 2·ζ·√(k·m)`. Convenient when you have a feel target (e.g. ζ=0.5)
    /// rather than a raw friction coefficient.
    ///
    /// Public API surface for the Compositor settings page's live readout; not yet
    /// called from compositor code.
    #[allow(dead_code)]
    pub fn from_zeta(zeta: f64, stiffness: f64, mass: f64) -> Self {
        let damping = 2.0 * zeta * (stiffness * mass).sqrt();
        Self {
            stiffness,
            damping,
            mass,
        }
    }
}

/// What this spring is animating. The reload path uses this to decide which `[…]`
/// section in `config.toml` to re-apply when the file changes — position springs pick
/// up `[spring]`, fade springs pick up `[wallpaper].fade_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpringKind {
    /// Drives a window position (or any value where physical-pixel feel matters).
    Position,
    /// Drives an opacity fade (e.g. wallpaper crossfade).
    Fade,
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

    /// Returns `true` if the spring is at rest — close to target by `SETTLE_EPSILON_POS`
    /// and moving slower than `SETTLE_EPSILON_VEL`. Position and velocity have different
    /// units, so they need different thresholds; using one number for both was the
    /// original bug.
    pub fn is_settled(&self) -> bool {
        (self.position - self.target).abs() < SETTLE_EPSILON_POS
            && self.velocity.abs() < SETTLE_EPSILON_VEL
    }

    /// Force the spring to its target exactly. Called by the engine once a spring
    /// crosses the settle threshold so it doesn't sit at `target + 0.4` indefinitely.
    pub fn snap_to_target(&mut self) {
        self.position = self.target;
        self.velocity = 0.0;
    }

    /// Integrate one symplectic-Euler step of length `dt`. Caller is responsible
    /// for keeping `dt` ≤ [`MAX_STEP`]; [`SpringState::tick`] does the sub-stepping
    /// and is the entry point everyone uses.
    fn step(&mut self, params: &SpringParams, dt: f64) {
        let displacement = self.position - self.target;
        let acceleration =
            (-params.stiffness * displacement - params.damping * self.velocity) / params.mass;
        self.velocity += acceleration * dt;
        self.position += self.velocity * dt;
    }

    /// Advance the spring by `dt` seconds, sub-stepping if needed to stay numerically
    /// stable on stiff systems. Returns nothing — read `position` / `velocity` after.
    pub fn tick(&mut self, params: &SpringParams, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        let n = (dt / MAX_STEP).ceil().max(1.0) as usize;
        let h = dt / n as f64;
        for _ in 0..n {
            self.step(params, h);
        }
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
    pub kind: SpringKind,
}

impl SpringHandle {
    /// Convenience constructor for a `SpringKind::Position` handle.
    #[allow(dead_code)] // tests use it; runtime path goes through with_kind via the engine
    pub fn new(id: SpringId, initial_x: i32, initial_y: i32, params: SpringParams) -> Self {
        Self::with_kind(id, initial_x, initial_y, params, SpringKind::Position)
    }

    pub fn with_kind(
        id: SpringId,
        initial_x: i32,
        initial_y: i32,
        params: SpringParams,
        kind: SpringKind,
    ) -> Self {
        Self {
            id,
            x: SpringState::new(initial_x as f64),
            y: SpringState::new(initial_y as f64),
            params,
            kind,
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
        self.x.is_settled() && self.y.is_settled()
    }

    /// Advance both axes by `dt` seconds. The engine ticks each axis independently
    /// (per-axis settling) so this is only used by tests that exercise a standalone
    /// handle outside the engine.
    #[allow(dead_code)]
    pub fn tick(&mut self, dt: f64) {
        self.x.tick(&self.params, dt);
        self.y.tick(&self.params, dt);
    }
}

/// Settle threshold for *position*: within half a pixel of target.
pub const SETTLE_EPSILON_POS: f64 = 0.5;
/// Settle threshold for *velocity*: less than 1 px/s. Looser than `SETTLE_EPSILON_POS`
/// because px/s is not the same unit as px, and a sub-1px/s residual is imperceptible
/// at any sensible frame rate.
pub const SETTLE_EPSILON_VEL: f64 = 1.0;

/// Maximum dt the caller is allowed to feed into a tick before clamping. Two frames
/// at 60 Hz; anything larger is assumed to be a dropped-frame burp.
pub const MAX_DT: Duration = Duration::from_millis(33);

/// Maximum sub-step inside `SpringState::tick`. ≈4 ms keeps stiffness up to ~10⁴ stable
/// for any incoming `dt` (after `MAX_DT` clamping at the engine level).
pub const MAX_STEP: f64 = 1.0 / 240.0;

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

        assert!(state.is_settled());
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

    #[test]
    fn snap_to_target_zeroes_state() {
        let mut state = SpringState::new(99.7);
        state.target = 100.0;
        state.velocity = 0.3;
        state.snap_to_target();
        assert_eq!(state.position, 100.0);
        assert_eq!(state.velocity, 0.0);
    }

    #[test]
    fn from_zeta_critically_damped() {
        let p = SpringParams::from_zeta(1.0, 100.0, 1.0);
        // ζ=1.0, k=100, m=1 → d = 2·1·√100 = 20.
        assert!((p.damping - 20.0).abs() < 1e-9);
    }

    #[test]
    fn stiff_spring_remains_stable_under_large_dt() {
        // Stiffness 2000 with dt = 33 ms (a dropped-frame burp) would explode under
        // a single Euler step; sub-stepping must keep it bounded.
        let params = SpringParams {
            stiffness: 2000.0,
            damping: 60.0,
            mass: 1.0,
        };
        let mut state = SpringState::new(0.0);
        state.target = 1000.0;

        let mut max_pos = 0.0f64;
        for _ in 0..60 {
            state.tick(&params, 0.033); // 33 ms
            max_pos = max_pos.max(state.position);
            assert!(state.position.is_finite(), "position blew up to NaN/inf");
        }
        // ζ ≈ 0.67 → some overshoot expected, but nowhere near divergence.
        assert!(max_pos < 1200.0, "overshoot exploded, max was {max_pos}");
        assert!((state.position - 1000.0).abs() < 1.0);
    }

    #[test]
    fn settle_thresholds_independent_for_velocity_and_position() {
        // Position close to target, but velocity above the velocity epsilon → not settled.
        let mut s = SpringState::new(100.0);
        s.target = 100.0;
        s.velocity = 5.0;
        assert!(!s.is_settled());
        s.velocity = 0.1;
        assert!(s.is_settled());
    }
}
