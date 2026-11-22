//! The DRM compositor uses synchronized flips. Accept per-surface low-latency
//! hints so clients can use the same presentation path on either backend.
use crate::state::State;
use smithay::{
    reexports::{
        wayland_protocols::wp::tearing_control::v1::server::{
            wp_tearing_control_manager_v1 as manager, wp_tearing_control_v1 as control,
        },
        wayland_server::{
            protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle,
            GlobalDispatch, New, Resource, WEnum,
        },
    },
    wayland::compositor::{with_states, Cacheable},
};
use std::cell::Cell;

#[derive(Default)]
struct Associated(Cell<bool>);
#[derive(Clone, Copy, Default)]
struct Hint {
    asynchronous: bool,
}
impl Cacheable for Hint {
    fn commit(&mut self, _: &DisplayHandle) -> Self {
        *self
    }
    fn merge_into(self, into: &mut Self, _: &DisplayHandle) {
        *into = self;
    }
}
impl GlobalDispatch<manager::WpTearingControlManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<manager::WpTearingControlManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}
impl Dispatch<manager::WpTearingControlManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &manager::WpTearingControlManagerV1,
        req: manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let manager::Request::GetTearingControl { id, surface } = req {
            let exists = with_states(&surface, |data| {
                data.data_map.insert_if_missing(Associated::default);
                data.data_map.get::<Associated>().unwrap().0.replace(true)
            });
            init.init(id, surface);
            if exists {
                resource.post_error(
                    manager::Error::TearingControlExists,
                    "surface already has a tearing control",
                );
            }
        }
    }
}
impl Dispatch<control::WpTearingControlV1, WlSurface> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &control::WpTearingControlV1,
        req: control::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if !surface.is_alive() {
            return;
        }
        with_states(surface, |data| {
            let mut hint = data.cached_state.get::<Hint>();
            match req {
                control::Request::SetPresentationHint {
                    hint: WEnum::Value(v),
                } => hint.pending().asynchronous = v == control::PresentationHint::Async,
                control::Request::Destroy => {
                    data.data_map.get::<Associated>().unwrap().0.set(false);
                    hint.pending().asynchronous = false;
                }
                _ => {}
            }
        });
    }
}
pub fn register(dh: &DisplayHandle) {
    dh.create_global::<State, manager::WpTearingControlManagerV1, _>(1, ());
}
