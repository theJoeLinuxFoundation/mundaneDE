//! The wallpaper engine.
//!
//! A wallpaper is a decoded image rendered behind every window. Its opacity is driven by a
//! spring (see [`crate::animation`]) — the project's defining idea — so a wallpaper *fades
//! in* on startup and *crossfades* when swapped. The manager keeps decoded pixels on the CPU
//! and lazily uploads them to a [`GlesTexture`] the first time a renderer is in hand, so it
//! stays independent of which backend is running (both use the [`GlesRenderer`]).
//!
//! During a crossfade two layers coexist: the incoming `current` (fading 0→100) drawn on top
//! of the outgoing `previous` (fading 100→0). The fade value lives on the spring's `x` axis,
//! read as `position / 100` and clamped to `[0, 1]`.

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform};

use crate::animation::{AnimationEngine, SpringId};
use crate::config::{ScalingMode, WallpaperSection};

/// A single decoded wallpaper image and the spring animating its opacity.
struct WallpaperLayer {
    /// Tightly packed RGBA8 pixels (`Fourcc::Abgr8888` byte order).
    pixels: Vec<u8>,
    /// Image dimensions in pixels.
    size: (i32, i32),
    /// GPU texture, imported lazily on first render and cached thereafter.
    texture: Option<TextureBuffer<GlesTexture>>,
    /// Opacity spring; `x.position` runs 0..=100 (→ alpha 0.0..=1.0).
    fade: SpringId,
}

impl WallpaperLayer {
    /// Upload the pixels to a GPU texture on first use; returns the cached buffer.
    fn texture(&mut self, renderer: &mut GlesRenderer) -> Option<&TextureBuffer<GlesTexture>> {
        if self.texture.is_none() {
            match TextureBuffer::from_memory(
                renderer,
                &self.pixels,
                Fourcc::Abgr8888,
                Size::<i32, Buffer>::from(self.size),
                false,
                1,
                Transform::Normal,
                None,
            ) {
                Ok(buffer) => self.texture = Some(buffer),
                Err(e) => {
                    tracing::warn!("wallpaper texture import failed: {e:?}");
                    return None;
                }
            }
        }
        self.texture.as_ref()
    }
}

/// Owns the active wallpaper (and the outgoing one mid-crossfade) and builds its render
/// elements each frame.
pub struct WallpaperManager {
    mode: ScalingMode,
    fade_params: crate::animation::SpringParams,
    current: Option<WallpaperLayer>,
    previous: Option<WallpaperLayer>,
}

impl WallpaperManager {
    /// Build the manager from config, decoding the configured image (if any) and starting its
    /// fade-in.
    pub fn new(config: &WallpaperSection, animation: &mut AnimationEngine) -> Self {
        let fade_params = config.to_fade_params();
        let current = config
            .path
            .as_ref()
            .and_then(|path| match decode(path) {
                Ok((pixels, size)) => Some(new_layer(pixels, size, fade_params, animation)),
                Err(e) => {
                    tracing::warn!("failed to load wallpaper '{path}': {e}");
                    None
                }
            });
        Self {
            mode: config.mode,
            fade_params,
            current,
            previous: None,
        }
    }

    /// Swap to a new wallpaper, crossfading from the current one. Decoding happens here (CPU
    /// only); the GPU upload is deferred to the next render.
    pub fn set_wallpaper(&mut self, path: &str, animation: &mut AnimationEngine) {
        let (pixels, size) = match decode(path) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("failed to load wallpaper '{path}': {e}");
                return;
            }
        };
        if let Some(current) = self.current.take() {
            if let Some(spring) = animation.spring_mut(current.fade) {
                spring.set_target(0, 0);
            }
            // Drop a still-fading older layer; only one crossfade runs at a time.
            if let Some(old) = self.previous.take() {
                animation.remove_spring(old.fade);
            }
            self.previous = Some(current);
        }
        self.current = Some(new_layer(pixels, size, self.fade_params, animation));
    }

    /// Drop the outgoing layer once its fade-out has settled, releasing its spring and texture.
    pub fn cleanup(&mut self, animation: &mut AnimationEngine) {
        let settled = self
            .previous
            .as_ref()
            .map(|p| {
                animation
                    .spring(p.fade)
                    .map(|s| s.x.position <= 0.5)
                    .unwrap_or(true)
            })
            .unwrap_or(false);
        if settled {
            if let Some(old) = self.previous.take() {
                animation.remove_spring(old.fade);
            }
        }
    }

    /// Build the wallpaper's render elements, back layer last (front-to-back ordering). The
    /// caller appends these *after* the window elements so the wallpaper sits behind them.
    pub fn elements(
        &mut self,
        renderer: &mut GlesRenderer,
        animation: &AnimationEngine,
        output: (i32, i32),
    ) -> Vec<TextureRenderElement<GlesTexture>> {
        let mode = self.mode;
        let mut elements = Vec::new();
        // `current` first → drawn on top of `previous` while crossfading in.
        if let Some(layer) = self.current.as_mut() {
            push_layer(layer, mode, output, renderer, animation, &mut elements);
        }
        if let Some(layer) = self.previous.as_mut() {
            push_layer(layer, mode, output, renderer, animation, &mut elements);
        }
        elements
    }
}

/// Decode a PNG/JPEG file to packed RGBA8 + dimensions.
fn decode(path: &str) -> anyhow::Result<(Vec<u8>, (i32, i32))> {
    let image = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?;
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((rgba.into_raw(), (w as i32, h as i32)))
}

/// Create a layer with a fresh fade-in spring (0 → 100).
fn new_layer(
    pixels: Vec<u8>,
    size: (i32, i32),
    params: crate::animation::SpringParams,
    animation: &mut AnimationEngine,
) -> WallpaperLayer {
    let fade = animation.create_spring(0, 0, params);
    if let Some(spring) = animation.spring_mut(fade) {
        spring.set_target(100, 0);
    }
    WallpaperLayer {
        pixels,
        size,
        texture: None,
        fade,
    }
}

/// Emit the render element(s) for one layer at its current fade alpha.
fn push_layer(
    layer: &mut WallpaperLayer,
    mode: ScalingMode,
    output: (i32, i32),
    renderer: &mut GlesRenderer,
    animation: &AnimationEngine,
    out: &mut Vec<TextureRenderElement<GlesTexture>>,
) {
    let alpha = animation
        .spring(layer.fade)
        .map(|s| (s.x.position / 100.0).clamp(0.0, 1.0) as f32)
        .unwrap_or(1.0);
    if alpha <= 0.001 {
        return;
    }
    let image = layer.size;
    let Some(buffer) = layer.texture(renderer) else {
        return;
    };
    for (location, src, size) in placements(mode, image, output) {
        out.push(TextureRenderElement::from_texture_buffer(
            location,
            buffer,
            Some(alpha),
            src,
            size,
            Kind::Unspecified,
        ));
    }
}

type Placement = (
    Point<f64, Physical>,
    Option<Rectangle<f64, Logical>>,
    Option<Size<i32, Logical>>,
);

/// Compute destination placement(s) for an image of `image` size on an `output` of the given
/// size, per the scaling mode. Output scale is 1, so logical == physical here.
fn placements(mode: ScalingMode, image: (i32, i32), output: (i32, i32)) -> Vec<Placement> {
    let (iw, ih) = (image.0 as f64, image.1 as f64);
    let (ow, oh) = (output.0 as f64, output.1 as f64);
    if iw <= 0.0 || ih <= 0.0 {
        return Vec::new();
    }

    match mode {
        ScalingMode::Stretch => vec![(
            Point::from((0.0, 0.0)),
            None,
            Some(Size::from((output.0, output.1))),
        )],
        ScalingMode::Fill => {
            // Cover the output, cropping a centred source region (preserves aspect).
            let scale = (ow / iw).max(oh / ih);
            let vis_w = (ow / scale).min(iw);
            let vis_h = (oh / scale).min(ih);
            let src = Rectangle::new(
                Point::from(((iw - vis_w) / 2.0, (ih - vis_h) / 2.0)),
                Size::from((vis_w, vis_h)),
            );
            vec![(
                Point::from((0.0, 0.0)),
                Some(src),
                Some(Size::from((output.0, output.1))),
            )]
        }
        ScalingMode::Fit => {
            // Fit inside the output, letterboxed (preserves aspect).
            let scale = (ow / iw).min(oh / ih);
            let dest_w = (iw * scale).round();
            let dest_h = (ih * scale).round();
            let loc = Point::from(((ow - dest_w) / 2.0, (oh - dest_h) / 2.0));
            vec![(
                loc,
                None,
                Some(Size::from((dest_w as i32, dest_h as i32))),
            )]
        }
        ScalingMode::Center => {
            // Original size, centred (may overflow and be clipped to the output).
            let loc = Point::from(((ow - iw) / 2.0, (oh - ih) / 2.0));
            vec![(loc, None, Some(Size::from((image.0, image.1))))]
        }
        ScalingMode::Tile => {
            // Original size, repeated to cover the output.
            let mut tiles = Vec::new();
            let mut y = 0;
            while y < output.1 {
                let mut x = 0;
                while x < output.0 {
                    tiles.push((
                        Point::from((x as f64, y as f64)),
                        None,
                        Some(Size::from((image.0, image.1))),
                    ));
                    x += image.0;
                }
                y += image.1;
            }
            tiles
        }
    }
}

#[cfg(test)]
mod tests {
    use super::placements;
    use crate::config::ScalingMode;

    #[test]
    fn stretch_fills_output_exactly() {
        let (loc, src, size) = placements(ScalingMode::Stretch, (800, 600), (1920, 1080))
            .into_iter()
            .next()
            .unwrap();
        assert_eq!((loc.x, loc.y), (0.0, 0.0));
        assert!(src.is_none());
        let size = size.unwrap();
        assert_eq!((size.w, size.h), (1920, 1080));
    }

    #[test]
    fn fill_crops_wide_source_and_covers_output() {
        // A 2:1 image into a 16:9 output crops left/right and covers the whole output.
        let (loc, src, size) = placements(ScalingMode::Fill, (4000, 2000), (1920, 1080))
            .into_iter()
            .next()
            .unwrap();
        assert_eq!((loc.x, loc.y), (0.0, 0.0));
        let size = size.unwrap();
        assert_eq!((size.w, size.h), (1920, 1080));
        let src = src.expect("fill should crop a source region");
        // scale = max(1920/4000, 1080/2000) = 0.54 → full height, cropped width.
        assert!(src.size.h >= 1999.0, "source height should be full: {}", src.size.h);
        assert!(src.size.w < 4000.0, "source width should be cropped: {}", src.size.w);
        assert!(src.loc.x > 0.0, "crop should be horizontally centred");
    }

    #[test]
    fn fit_letterboxes_inside_output() {
        // A square image into a 16:9 output fits by height, with side bars.
        let (loc, _src, size) = placements(ScalingMode::Fit, (1000, 1000), (1920, 1080))
            .into_iter()
            .next()
            .unwrap();
        let size = size.unwrap();
        assert_eq!((size.w, size.h), (1080, 1080));
        assert!(loc.x > 0.0, "should be centred with side bars");
        assert!(loc.y.abs() < 1.0, "fits vertically, no top bar");
    }

    #[test]
    fn tile_repeats_to_cover_output() {
        // ceil(250/100) = 3 across, ceil(150/100) = 2 down → 6 tiles.
        let tiles = placements(ScalingMode::Tile, (100, 100), (250, 150));
        assert_eq!(tiles.len(), 6);
    }
}
