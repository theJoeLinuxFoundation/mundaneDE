//! Pure magnetic-snapping geometry.
//!
//! [`snap`] computes where a window's top-left corner should land after a drag, given
//! the output bounds and the rectangles of the other windows. A window edge that comes
//! within `attraction_radius` of an output edge or a neighbour edge is pulled flush to
//! it, leaving a `gap_size` margin. The two axes are independent, so a window can snap
//! into a corner (x to one thing, y to another).
//!
//! This module is deliberately free of any compositor state — it operates only on the
//! plain geometry types from [`smithay::utils`], so the behaviour is fully unit-testable
//! without a running compositor.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Compute the magnetic snap target for a window's top-left corner.
///
/// * `desired` — the window's top-left after the drag (where the pointer left it).
/// * `size` — the window's size.
/// * `output` — the usable output size.
/// * `neighbors` — geometries of all *other* windows.
/// * `attraction_radius` — max distance (px) at which an edge is pulled to a target.
/// * `gap_size` — margin (px) left between snapped edges.
pub fn snap(
    desired: Point<i32, Logical>,
    size: Size<i32, Logical>,
    output: Size<i32, Logical>,
    neighbors: &[Rectangle<i32, Logical>],
    attraction_radius: i32,
    gap_size: i32,
) -> Point<i32, Logical> {
    let x = snap_axis(
        desired.x,
        size.w,
        output.w,
        neighbors.iter().map(|r| (r.loc.x, r.size.w)),
        attraction_radius,
        gap_size,
    );
    let y = snap_axis(
        desired.y,
        size.h,
        output.h,
        neighbors.iter().map(|r| (r.loc.y, r.size.h)),
        attraction_radius,
        gap_size,
    );
    Point::from((x, y))
}

/// Snap one axis.
///
/// `p` is the window-start (left or top), `len` the window length on this axis, `out`
/// the output length, and each neighbour is `(start, length)` on this axis. Because the
/// window is rigid, moving its start by Δ moves every edge by Δ, so the distance for any
/// candidate target position `c` is simply `|p - c|`. We collect all candidate positions,
/// keep those within `radius`, and pick the closest. Output edges are considered first,
/// so they win ties (a slight bias toward the screen edges). If nothing is in range, the
/// window stays where it was dropped.
fn snap_axis(
    p: i32,
    len: i32,
    out: i32,
    neighbors: impl Iterator<Item = (i32, i32)>,
    radius: i32,
    gap: i32,
) -> i32 {
    let mut best: Option<(i32, i32)> = None; // (distance, candidate position)
    let mut consider = |c: i32| {
        let d = (p - c).abs();
        if d <= radius {
            match best {
                Some((bd, _)) if bd <= d => {}
                _ => best = Some((d, c)),
            }
        }
    };

    // Output edges (considered before neighbours → screen-edge bias on ties).
    consider(gap); // leading edge flush to output start + gap
    consider(out - gap - len); // trailing edge flush to output end - gap

    for (n_start, n_len) in neighbors {
        consider(n_start + n_len + gap); // our leading edge to neighbour's trailing edge + gap
        consider(n_start - gap - len); // our trailing edge to neighbour's leading edge - gap
    }

    best.map(|(_, c)| c).unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: (i32, i32) = (1000, 800);
    const RADIUS: i32 = 24;
    const GAP: i32 = 8;

    fn out() -> Size<i32, Logical> {
        Size::from(OUT)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    #[test]
    fn snaps_to_left_edge() {
        // Window 5px from the left → leading edge within radius of `gap`.
        let r = snap(Point::from((5, 300)), Size::from((200, 200)), out(), &[], RADIUS, GAP);
        assert_eq!(r.x, GAP);
    }

    #[test]
    fn snaps_to_right_edge() {
        // Trailing edge near output right: place trailing edge at out.w - gap.
        let size = Size::from((200, 200));
        let desired_x = OUT.0 - GAP - size.w + 5; // 5px inside the snap point
        let r = snap(Point::from((desired_x, 300)), size, out(), &[], RADIUS, GAP);
        assert_eq!(r.x, OUT.0 - GAP - size.w);
    }

    #[test]
    fn snaps_to_top_edge() {
        let r = snap(Point::from((400, 6)), Size::from((200, 200)), out(), &[], RADIUS, GAP);
        assert_eq!(r.y, GAP);
    }

    #[test]
    fn snaps_to_bottom_edge() {
        let size = Size::from((200, 200));
        let desired_y = OUT.1 - GAP - size.h - 4;
        let r = snap(Point::from((400, desired_y)), size, out(), &[], RADIUS, GAP);
        assert_eq!(r.y, OUT.1 - GAP - size.h);
    }

    #[test]
    fn no_snap_beyond_radius() {
        // Dead centre, far from every edge and no neighbours → unchanged.
        let desired = Point::from((400, 300));
        let r = snap(desired, Size::from((200, 200)), out(), &[], RADIUS, GAP);
        assert_eq!((r.x, r.y), (400, 300));
    }

    #[test]
    fn neighbor_snap_with_gap_left_side() {
        // Neighbour occupies x in [100, 300). Our left edge should snap to 300 + gap.
        let neighbor = rect(100, 0, 200, 200); // right edge at 300
        let desired_x = 300 + GAP + 3; // just inside snap range of the neighbour's right edge
        let r = snap(Point::from((desired_x, 300)), Size::from((150, 150)), out(), &[neighbor], RADIUS, GAP);
        assert_eq!(r.x, 300 + GAP);
        // Resulting gap between neighbour right edge (300) and our left edge equals GAP.
        assert_eq!(r.x - 300, GAP);
    }

    #[test]
    fn neighbor_snap_right_side() {
        // Neighbour starts at x = 600. Our trailing edge should snap to 600 - gap.
        let neighbor = rect(600, 0, 200, 200);
        let size = Size::from((150, 150));
        let desired_x = 600 - GAP - size.w - 5;
        let r = snap(Point::from((desired_x, 300)), size, out(), &[neighbor], RADIUS, GAP);
        assert_eq!(r.x, 600 - GAP - size.w);
        // Gap between our right edge and neighbour's left edge equals GAP.
        assert_eq!(600 - (r.x + size.w), GAP);
    }

    #[test]
    fn corner_both_axes() {
        // Near the top edge (y snaps to output top) and to the right of a neighbour
        // (x snaps to the neighbour's right edge + gap).
        let neighbor = rect(100, 0, 200, 600); // right edge at 300
        let r = snap(Point::from((300 + GAP + 4, 6)), Size::from((150, 150)), out(), &[neighbor], RADIUS, GAP);
        assert_eq!(r.x, 300 + GAP);
        assert_eq!(r.y, GAP);
    }

    #[test]
    fn nearest_of_multiple_wins() {
        // Two neighbours offer snap candidates at different distances; the closer wins.
        // Neighbour A right edge at 300 → candidate 300+gap=308. Neighbour B right edge
        // at 350 → candidate 358. Desired left edge at 356 (2px from 358, 48px from 308).
        let a = rect(100, 0, 200, 200); // right edge 300
        let b = rect(150, 400, 200, 200); // right edge 350
        let r = snap(Point::from((356, 300)), Size::from((100, 100)), out(), &[a, b], RADIUS, GAP);
        assert_eq!(r.x, 358);
    }

    #[test]
    fn output_edge_preferred_on_tie() {
        // Construct an exact tie between the output-left candidate (gap=8) and a
        // neighbour candidate, both `d` away from `p`. Output edge must win.
        // Output-left candidate = 8. Neighbour right edge n → candidate n+gap.
        // Choose p so |p-8| == |p-(n+gap)|. p=20: |20-8|=12. Need n+gap=32 → n=24.
        let neighbor = rect(0, 400, 24, 100); // right edge at 24 → candidate 32
        let r = snap(Point::from((20, 300)), Size::from((100, 100)), out(), &[neighbor], RADIUS, GAP);
        assert_eq!(r.x, GAP); // output edge wins the tie
    }

    #[test]
    fn large_window_clamps_left() {
        // Window wider than the usable area: left candidate (gap) is closest, no panic.
        let big = Size::from((OUT.0 + 200, 100));
        let r = snap(Point::from((3, 300)), big, out(), &[], RADIUS, GAP);
        assert_eq!(r.x, GAP);
    }

    #[test]
    fn radius_boundary_is_inclusive() {
        // Exactly `radius` away → snaps. `radius + 1` away → does not.
        let at = snap(Point::from((GAP + RADIUS, 300)), Size::from((100, 100)), out(), &[], RADIUS, GAP);
        assert_eq!(at.x, GAP);
        let beyond = snap(Point::from((GAP + RADIUS + 1, 300)), Size::from((100, 100)), out(), &[], RADIUS, GAP);
        assert_eq!(beyond.x, GAP + RADIUS + 1);
    }
}
