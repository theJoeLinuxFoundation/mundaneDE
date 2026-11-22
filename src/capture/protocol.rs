//! SHM is the common buffer type on software-rendered guests and DRM sessions.
use super::{Pending, Source, State};
use crate::shell::services::{role, Role};
use smithay::{
    output::Output,
    reexports::{
        wayland_protocols::ext::{
            image_capture_source::v1::server::{
                ext_foreign_toplevel_image_capture_source_manager_v1 as window,
                ext_image_capture_source_v1 as source,
                ext_output_image_capture_source_manager_v1 as output,
            },
            image_copy_capture::v1::server::{
                ext_image_copy_capture_cursor_session_v1 as cursor,
                ext_image_copy_capture_frame_v1 as frame,
                ext_image_copy_capture_manager_v1 as manager,
                ext_image_copy_capture_session_v1 as session,
            },
        },
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1 as legacy, zwlr_screencopy_manager_v1 as wlr,
        },
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_output, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum, Weak,
        },
    },
    utils::{Physical, Rectangle, Size},
    wayland::foreign_toplevel_list::ForeignToplevelHandle,
};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

type Session = session::ExtImageCopyCaptureSessionV1;
type ExtFrame = frame::ExtImageCopyCaptureFrameV1;
type WlrFrame = legacy::ZwlrScreencopyFrameV1;
pub(super) enum Frame {
    Ext(ExtFrame, Session),
    Wlr(WlrFrame),
}

impl Frame {
    pub fn alive(&self) -> bool {
        match self {
            Self::Ext(f, s) => f.is_alive() && s.is_alive(),
            Self::Wlr(f) => f.is_alive(),
        }
    }
    pub fn resized(&self, size: Size<i32, Physical>) -> bool {
        if let Self::Ext(f, s) = self {
            let data = s.data::<CaptureSession>().unwrap();
            if *data.size.lock().unwrap() != Some(size) {
                constraints(s, Some(size));
                f.failed(frame::FailureReason::BufferConstraints);
                return true;
            }
        }
        false
    }
    pub fn fail(&self) {
        match self {
            Self::Ext(f, _) => f.failed(frame::FailureReason::Unknown),
            Self::Wlr(f) => f.failed(),
        }
    }
    pub fn ready(
        &self,
        buffer: &WlBuffer,
        generation: u64,
        time: std::time::Duration,
        size: Size<i32, Physical>,
    ) {
        let (hi, lo, ns) = (
            (time.as_secs() >> 32) as u32,
            time.as_secs() as u32,
            time.subsec_nanos(),
        );
        match self {
            Self::Ext(f, s) => {
                let mut generations = s
                    .data::<CaptureSession>()
                    .unwrap()
                    .generations
                    .lock()
                    .unwrap();
                generations.retain(|buffer, _| buffer.is_alive());
                generations.insert(buffer.downgrade(), generation);
                f.transform(wl_output::Transform::Normal);
                f.damage(0, 0, size.w, size.h);
                f.presentation_time(hi, lo, ns);
                f.ready();
            }
            Self::Wlr(f) => {
                f.flags(legacy::Flags::empty());
                f.ready(hi, lo, ns);
            }
        }
    }
}

macro_rules! capture_global {
    ($ty:ty,$role:pat) => {
        impl GlobalDispatch<$ty, ()> for State {
            fn bind(
                _: &mut Self,
                _: &DisplayHandle,
                _: &Client,
                new: New<$ty>,
                _: &(),
                init: &mut DataInit<'_, Self>,
            ) {
                init.init(new, ());
            }
            fn can_view(client: Client, _: &()) -> bool {
                matches!(role(&client), $role)
            }
        }
    };
}
capture_global!(output::ExtOutputImageCaptureSourceManagerV1, Role::Capture);
capture_global!(
    window::ExtForeignToplevelImageCaptureSourceManagerV1,
    Role::Capture
);
capture_global!(manager::ExtImageCopyCaptureManagerV1, Role::Capture);
capture_global!(wlr::ZwlrScreencopyManagerV1, Role::Screenshot);
pub fn register(d: &DisplayHandle) {
    d.create_global::<State, output::ExtOutputImageCaptureSourceManagerV1, _>(1, ());
    d.create_global::<State, window::ExtForeignToplevelImageCaptureSourceManagerV1, _>(1, ());
    d.create_global::<State, manager::ExtImageCopyCaptureManagerV1, _>(1, ());
    d.create_global::<State, wlr::ZwlrScreencopyManagerV1, _>(1, ());
}
impl State {
    fn allowed_source(&self, c: &Client, s: Option<Source>) -> Option<Source> {
        s.filter(|s| !self.desktop.lock.active && self.capture.grants.get(&c.id()) == Some(s))
    }
}

impl Dispatch<output::ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn request(
        st: &mut Self,
        c: &Client,
        _: &output::ExtOutputImageCaptureSourceManagerV1,
        r: output::Request,
        _: &(),
        _: &DisplayHandle,
        i: &mut DataInit<'_, Self>,
    ) {
        if let output::Request::CreateSource { source, output } = r {
            i.init(
                source,
                st.allowed_source(
                    c,
                    Output::from_resource(&output).map(|o| Source::Output(o.name())),
                ),
            );
        }
    }
}
impl Dispatch<window::ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn request(
        st: &mut Self,
        c: &Client,
        _: &window::ExtForeignToplevelImageCaptureSourceManagerV1,
        r: window::Request,
        _: &(),
        _: &DisplayHandle,
        i: &mut DataInit<'_, Self>,
    ) {
        if let window::Request::CreateSource {
            source,
            toplevel_handle,
        } = r
        {
            i.init(
                source,
                st.allowed_source(
                    c,
                    ForeignToplevelHandle::from_resource(&toplevel_handle)
                        .map(|t| Source::Window(t.identifier())),
                ),
            );
        }
    }
}
impl Dispatch<source::ExtImageCaptureSourceV1, Option<Source>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &source::ExtImageCaptureSourceV1,
        _: source::Request,
        _: &Option<Source>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
#[derive(Default)]
struct CaptureSession {
    source: Option<Source>,
    cursor: bool,
    frame: AtomicBool,
    // Prime every new PipeWire buffer even when the selected source is static.
    generations: Mutex<HashMap<Weak<WlBuffer>, u64>>,
    size: Mutex<Option<Size<i32, Physical>>>,
}
fn constraints(s: &Session, size: Option<Size<i32, Physical>>) {
    if let Some(sz) = size {
        let data = s.data::<CaptureSession>().unwrap();
        *data.size.lock().unwrap() = Some(sz);
        data.generations.lock().unwrap().clear();
        s.buffer_size(sz.w as u32, sz.h as u32);
        s.shm_format(wl_shm::Format::Xrgb8888);
        s.done();
    } else {
        s.stopped();
    }
}
impl Dispatch<manager::ExtImageCopyCaptureManagerV1, ()> for State {
    fn request(
        st: &mut Self,
        _: &Client,
        m: &manager::ExtImageCopyCaptureManagerV1,
        r: manager::Request,
        _: &(),
        _: &DisplayHandle,
        i: &mut DataInit<'_, Self>,
    ) {
        match r {
            manager::Request::CreateSession {
                session,
                source,
                options,
            } => {
                let src = source.data::<Option<Source>>().cloned().flatten();
                let size = src.as_ref().and_then(|s| st.capture_size(s));
                let WEnum::Value(options) = options else {
                    m.post_error(manager::Error::InvalidOption, "unknown capture options");
                    return;
                };
                let s = i.init(
                    session,
                    CaptureSession {
                        source: src,
                        cursor: options.contains(manager::Options::PaintCursors),
                        ..Default::default()
                    },
                );
                constraints(&s, size);
            }
            // The portal advertises hidden/embedded cursors. A separate cursor stream stays empty.
            manager::Request::CreatePointerCursorSession { session, .. } => {
                i.init(session, AtomicBool::new(false));
            }
            _ => {}
        }
    }
}
impl Dispatch<cursor::ExtImageCopyCaptureCursorSessionV1, AtomicBool> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        c: &cursor::ExtImageCopyCaptureCursorSessionV1,
        r: cursor::Request,
        used: &AtomicBool,
        _: &DisplayHandle,
        i: &mut DataInit<'_, Self>,
    ) {
        if let cursor::Request::GetCaptureSession { session } = r {
            if used.swap(true, Ordering::Relaxed) {
                c.post_error(cursor::Error::DuplicateSession, "duplicate cursor session");
                return;
            }
            i.init(session, CaptureSession::default()).stopped();
        }
    }
}
#[derive(Default)]
struct BufferState {
    buffer: Mutex<Option<WlBuffer>>,
    used: AtomicBool,
}
impl Dispatch<Session, CaptureSession> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        s: &Session,
        r: session::Request,
        data: &CaptureSession,
        _: &DisplayHandle,
        i: &mut DataInit<'_, Self>,
    ) {
        if let session::Request::CreateFrame { frame } = r {
            if data.frame.swap(true, Ordering::Relaxed) {
                s.post_error(session::Error::DuplicateFrame, "frame already exists");
                return;
            }
            i.init(frame, (s.clone(), BufferState::default()));
        }
    }
}
impl Dispatch<ExtFrame, (Session, BufferState)> for State {
    fn request(
        st: &mut Self,
        _: &Client,
        f: &ExtFrame,
        r: frame::Request,
        (s, b): &(Session, BufferState),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if matches!(r, frame::Request::Destroy) {
            s.data::<CaptureSession>()
                .unwrap()
                .frame
                .store(false, Ordering::Relaxed);
            return;
        }
        if b.used.load(Ordering::Relaxed) {
            f.post_error(frame::Error::AlreadyCaptured, "frame already submitted");
            return;
        }
        match r {
            frame::Request::AttachBuffer { buffer } => *b.buffer.lock().unwrap() = Some(buffer),
            frame::Request::DamageBuffer {
                x,
                y,
                width,
                height,
            } if x < 0 || y < 0 || width <= 0 || height <= 0 => {
                f.post_error(frame::Error::InvalidBufferDamage, "invalid damage")
            }
            frame::Request::Capture => {
                b.used.store(true, Ordering::Relaxed);
                let Some(buffer) = b.buffer.lock().unwrap().take() else {
                    f.post_error(frame::Error::NoBuffer, "no buffer attached");
                    return;
                };
                let data = s.data::<CaptureSession>().unwrap();
                let Some(source) = data.source.clone() else {
                    f.failed(frame::FailureReason::Stopped);
                    return;
                };
                let size = st.capture_size(&source);
                if size.is_none() || size != *data.size.lock().unwrap() {
                    constraints(s, size);
                    f.failed(frame::FailureReason::BufferConstraints);
                    return;
                }
                let after = data
                    .generations
                    .lock()
                    .unwrap()
                    .get(&buffer.downgrade())
                    .copied();
                st.capture.pending.push(Pending {
                    source,
                    buffer,
                    cursor: data.cursor,
                    region: Rectangle::from_size(size.unwrap()),
                    frame: Frame::Ext(f.clone(), s.clone()),
                    after: after.map(|generation| {
                        (
                            generation,
                            std::time::Instant::now() + std::time::Duration::from_secs(1),
                        )
                    }),
                });
            }
            _ => {}
        }
    }
}
struct LegacyData {
    source: Option<Source>,
    region: Rectangle<i32, Physical>,
    cursor: bool,
    used: AtomicBool,
}
impl Dispatch<wlr::ZwlrScreencopyManagerV1, ()> for State {
    fn request(
        st: &mut Self,
        c: &Client,
        _: &wlr::ZwlrScreencopyManagerV1,
        r: wlr::Request,
        _: &(),
        _: &DisplayHandle,
        i: &mut DataInit<'_, Self>,
    ) {
        let (new, out, cursor, crop) = match r {
            wlr::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, output, overlay_cursor, None),
            wlr::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (
                frame,
                output,
                overlay_cursor,
                Some(Rectangle::<i32, smithay::utils::Logical>::new(
                    (x, y).into(),
                    (width, height).into(),
                )),
            ),
            _ => return,
        };
        let o = Output::from_resource(&out);
        let src = st.allowed_source(c, o.as_ref().map(|o| Source::Output(o.name())));
        let full = src
            .as_ref()
            .and_then(|s| st.capture_size(s))
            .map(Rectangle::from_size);
        let area = full.and_then(|f| match crop {
            Some(r) => f.intersection(
                r.to_physical_precise_round(o.as_ref().unwrap().current_scale().fractional_scale()),
            ),
            None => Some(f),
        });
        let f = i.init(
            new,
            LegacyData {
                source: src.filter(|_| area.is_some()),
                region: area.unwrap_or_default(),
                cursor: cursor != 0,
                used: AtomicBool::new(false),
            },
        );
        if let Some(r) = area {
            f.buffer(
                wl_shm::Format::Xrgb8888,
                r.size.w as u32,
                r.size.h as u32,
                r.size.w as u32 * 4,
            );
        } else {
            f.failed();
        }
    }
}
impl Dispatch<WlrFrame, LegacyData> for State {
    fn request(
        st: &mut Self,
        _: &Client,
        f: &WlrFrame,
        r: legacy::Request,
        d: &LegacyData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let legacy::Request::Copy { buffer } = r else {
            return;
        };
        if d.used.swap(true, Ordering::Relaxed) {
            f.post_error(legacy::Error::AlreadyUsed, "frame already submitted");
            return;
        }
        if let Some(src) = d.source.clone() {
            st.capture.pending.push(Pending {
                source: src,
                buffer,
                cursor: d.cursor,
                region: d.region,
                frame: Frame::Wlr(f.clone()),
                after: None,
            });
        }
    }
}
