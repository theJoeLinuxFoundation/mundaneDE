//! Pure magnetic-snapping geometry.
//!
//! [`snap`] computes where a window's top-left corner should land after a drag, given
//! the output bounds and the rectangles of the other windows. A window edge — or its
//! centre line — that comes within `attraction_radius` of an output edge, a neighbour
//! edge, or the corresponding centre is pulled to it. Two gaps are honoured:
//!
//! * `outer_gap` — space between a snapped window and the output edge,
//! * `inner_gap` — space between two snapped windows.
//!
//! The two axes are independent, so a window can snap into a corner (x to one thing,
//! y to another).
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
/// * `attraction_radius` — max distance (px) at which a candidate is pulled in.
/// * `outer_gap` — margin between a snapped edge and the output edge.
/// * `inner_gap` — margin between two snapped windows.
pub fn snap(
    desired: Point<i32, Logical>,
    size: Size<i32, Logical>,
    output: Size<i32, Logical>,
    neighbors: &[Rectangle<i32, Logical>],
    attraction_radius: i32,
    outer_gap: i32,
    inner_gap: i32,
) -> Point<i32, Logical> {
    let x = snap_axis(
        desired.x,
        size.w,
        output.w,
        neighbors.iter().map(|r| (r.loc.x, r.size.w)),
        attraction_radius,
        outer_gap,
        inner_gap,
    );
    let y = snap_axis(
        desired.y,
        size.h,
        output.h,
        neighbors.iter().map(|r| (r.loc.y, r.size.h)),
        attraction_radius,
        outer_gap,
        inner_gap,
    );
    Point::from((x, y))
}

/// Snap one axis.
///
/// `p` is the window-start (left or top), `len` the window length on this axis, `out`
/// the output length, and each neighbour is `(start, length)` on this axis. The
/// candidate positions are:
///
/// * Output edges: leading edge flush to `outer_gap`, trailing edge flush to `out - outer_gap`.
/// * Output centre: window centre aligned to output centre.
/// * Per-neighbour: leading-edge-to-neighbour-trailing-edge + `inner_gap`,
///   trailing-edge-to-neighbour-leading-edge − `inner_gap`, and centre-to-centre.
///
/// Output edges are considered first, so they win ties (a slight bias toward the screen
/// edges). If nothing is within `radius`, the window stays where it was dropped.
fn snap_axis(
    p: i32,
    len: i32,
    out: i32,
    neighbors: impl Iterator<Item = (i32, i32)>,
    radius: i32,
    outer_gap: i32,
    inner_gap: i32,
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

    // Output edges (considered first → screen-edge bias on ties).
    consider(outer_gap); // leading edge flush to output start + outer_gap
    consider(out - outer_gap - len); // trailing edge flush to output end - outer_gap
    // Output centreline (window centre = output centre).
    consider((out - len) / 2);

    for (n_start, n_len) in neighbors {
        consider(n_start + n_len + inner_gap); // our leading edge to neighbour's trailing edge + inner_gap
        consider(n_start - inner_gap - len); // our trailing edge to neighbour's leading edge - inner_gap
        // Neighbour centreline (our centre aligned to neighbour's centre).
        consider(n_start + n_len / 2 - len / 2);
    }

    best.map(|(_, c)| c).unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: (i32, i32) = (1000, 800);
    const RADIUS: i32 = 24;
    const OUTER: i32 = 8;
    const INNER: i32 = 8;

    fn out() -> Size<i32, Logical> {
        Size::from(OUT)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((w, h)))
    }

    fn s(desired: Point<i32, Logical>, size: Size<i32, Logical>, neighbors: &[Rectangle<i32, Logical>]) -> Point<i32, Logical> {
        snap(desired, size, out(), neighbors, RADIUS, OUTER, INNER)
    }

    #[test]
    fn snaps_to_left_edge() {
        let r = s(Point::from((5, 300)), Size::from((200, 200)), &[]);
        assert_eq!(r.x, OUTER);
    }

    #[test]
    fn snaps_to_right_edge() {
        let size = Size::from((200, 200));
        let desired_x = OUT.0 - OUTER - size.w + 5;
        let r = s(Point::from((desired_x, 300)), size, &[]);
        assert_eq!(r.x, OUT.0 - OUTER - size.w);
    }

    #[test]
    fn snaps_to_top_edge() {
        let r = s(Point::from((400, 6)), Size::from((200, 200)), &[]);
        assert_eq!(r.y, OUTER);
    }

    #[test]
    fn snaps_to_bottom_edge() {
        let size = Size::from((200, 200));
        let desired_y = OUT.1 - OUTER - size.h - 4;
        let r = s(Point::from((400, desired_y)), size, &[]);
        assert_eq!(r.y, OUT.1 - OUTER - size.h);
    }

    #[test]
    fn no_snap_beyond_radius() {
        // Dead centre is the new "centre" candidate — pick a desired far enough from
        // every candidate that nothing wins, to test the no-snap fallback.
        let size = Size::from((200, 200));
        let desired = Point::from((350, 250)); // 100+ px from centre, edges, etc.
        let r = s(desired, size, &[]);
        assert_eq!((r.x, r.y), (350, 250));
    }

    #[test]
    fn neighbor_snap_with_inner_gap_left_side() {
        // Neighbour at x in [100, 300). Our left edge should snap to 300 + INNER.
        let neighbor = rect(100, 0, 200, 200);
        let desired_x = 300 + INNER + 3;
        let r = s(Point::from((desired_x, 300)), Size::from((150, 150)), &[neighbor]);
        assert_eq!(r.x, 300 + INNER);
        assert_eq!(r.x - 300, INNER);
    }

    #[test]
    fn neighbor_snap_right_side() {
        let neighbor = rect(600, 0, 200, 200);
        let size = Size::from((150, 150));
        let desired_x = 600 - INNER - size.w - 5;
        let r = s(Point::from((desired_x, 300)), size, &[neighbor]);
        assert_eq!(r.x, 600 - INNER - size.w);
        assert_eq!(600 - (r.x + size.w), INNER);
    }

    #[test]
    fn corner_both_axes() {
        let neighbor = rect(100, 0, 200, 600);
        let r = s(Point::from((300 + INNER + 4, 6)), Size::from((150, 150)), &[neighbor]);
        assert_eq!(r.x, 300 + INNER);
        assert_eq!(r.y, OUTER);
    }

    #[test]
    fn nearest_of_multiple_wins() {
        let a = rect(100, 0, 200, 200);
        let b = rect(150, 400, 200, 200);
        let r = s(Point::from((356, 300)), Size::from((100, 100)), &[a, b]);
        assert_eq!(r.x, 358); // closer candidate (b's right edge + INNER) wins
    }

    #[test]
    fn output_edge_preferred_on_tie() {
        // Construct an exact tie between the output-left candidate (8) and a neighbour
        // candidate; output edge must win.
        let neighbor = rect(0, 400, 24, 100); // right edge at 24 → candidate 24+INNER=32
        let r = s(Point::from((20, 300)), Size::from((100, 100)), &[neighbor]);
        assert_eq!(r.x, OUTER);
    }

    #[test]
    fn large_window_clamps_left() {
        let big = Size::from((OUT.0 + 200, 100));
        let r = s(Point::from((3, 300)), big, &[]);
        assert_eq!(r.x, OUTER);
    }

    #[test]
    fn radius_boundary_is_inclusive() {
        let size = Size::from((100, 100));
        let at = s(Point::from((OUTER + RADIUS, 300)), size, &[]);
        assert_eq!(at.x, OUTER);
        let beyond = s(Point::from((OUTER + RADIUS + 1, 300)), size, &[]);
        assert_eq!(beyond.x, OUTER + RADIUS + 1);
    }

    // ---------- centreline tests (new in this round of polish) ----------

    #[test]
    fn snaps_to_output_center() {
        // 200-wide window on a 1000-wide output → centred at x = (1000 - 200) / 2 = 400.
        // Drop near centre and expect snap to centre.
        let size = Size::from((200, 200));
        let center_target = (OUT.0 - size.w) / 2; // 400
        let r = s(Point::from((center_target + 5, 300)), size, &[]);
        assert_eq!(r.x, center_target);
    }

    #[test]
    fn snaps_to_neighbor_center() {
        // Neighbour at [100, 300) on x (centre at x=200). Our 100-wide window's
        // centre aligned to neighbour's centre → x = 200 - 50 = 150.
        let neighbor = rect(100, 0, 200, 200);
        let size = Size::from((100, 100));
        let neighbor_center = 100 + 200 / 2 - size.w / 2; // 150
        let r = s(Point::from((neighbor_center + 4, 400)), size, &[neighbor]);
        assert_eq!(r.x, neighbor_center);
    }

    #[test]
    fn edge_beats_center_when_closer() {
        // Window of 100 px. Output-left candidate is x=8. Output centre would be x=450.
        // Drop near the edge — the edge must win, even though both are valid candidates.
        let size = Size::from((100, 100));
        let r = s(Point::from((10, 300)), size, &[]);
        assert_eq!(r.x, OUTER);
    }
}
