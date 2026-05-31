//! A built-in mouse cursor.
//!
//! The DRM backend has no host compositor to draw a pointer, so we render our own. When a
//! client provides a cursor surface we render that; otherwise we fall back to this small
//! procedurally-drawn arrow so there is always a visible pointer. The hotspot is the
//! top-left tip at (0, 0).

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::utils::Transform;

const WIDTH: i32 = 14;
const HEIGHT: i32 = 21;

/// The arrow shape: `X` = black outline, `W` = white fill, `.` = transparent.
const ARROW: [&str; HEIGHT as usize] = [
    "X.............",
    "XX............",
    "XWX...........",
    "XWWX..........",
    "XWWWX.........",
    "XWWWWX........",
    "XWWWWWX.......",
    "XWWWWWWX......",
    "XWWWWWWWX.....",
    "XWWWWWWWWX....",
    "XWWWWWWWWWX...",
    "XWWWWWWWWWWX..",
    "XWWWWWWWWWWWX.",
    "XWWWWWWWWWWWWX",
    "XWWWWWWXXXXXXX",
    "XWWWXWWX......",
    "XWXX.XWWX.....",
    "XX...XWWX.....",
    ".....XWWX.....",
    "......XWX.....",
    "......XXX.....",
];

/// Build the built-in cursor as an RGBA memory buffer.
pub fn build_cursor() -> MemoryRenderBuffer {
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for row in ARROW {
        let bytes = row.as_bytes();
        for x in 0..WIDTH as usize {
            let rgba = match bytes.get(x).copied().unwrap_or(b'.') {
                b'X' => [0, 0, 0, 255],
                b'W' => [255, 255, 255, 255],
                _ => [0, 0, 0, 0],
            };
            pixels.extend_from_slice(&rgba);
        }
    }
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Abgr8888,
        (WIDTH, HEIGHT),
        1,
        Transform::Normal,
        None,
    )
}
