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

use smithay::input::pointer::CursorIcon;
use smithay::utils::{Logical, Point};

pub struct NamedCursor {
    pub icon: CursorIcon,
    pub buffer: MemoryRenderBuffer,
    pub hotspot: Point<i32, Logical>,
}

impl NamedCursor {
    pub fn load(icon: CursorIcon) -> Self {
        let theme = xcursor::CursorTheme::load(
            &std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into()),
        );
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        let image = std::iter::once(icon.name())
            .chain(icon.alt_names().iter().copied())
            .find_map(|name| theme.load_icon(name))
            .and_then(|path| std::fs::File::open(path).ok())
            .and_then(|file| {
                xcursor::parser::parse_xcursor_stream(&mut std::io::BufReader::new(file)).ok()
            })
            .and_then(|images| images.into_iter().min_by_key(|img| img.size.abs_diff(size)));
        if let Some(image) = image {
            Self {
                icon,
                buffer: MemoryRenderBuffer::from_slice(
                    &image.pixels_rgba,
                    Fourcc::Abgr8888,
                    (image.width as i32, image.height as i32),
                    1,
                    Transform::Normal,
                    None,
                ),
                hotspot: (image.xhot as i32, image.yhot as i32).into(),
            }
        } else {
            Self {
                icon,
                buffer: build_cursor(),
                hotspot: (0, 0).into(),
            }
        }
    }
}
