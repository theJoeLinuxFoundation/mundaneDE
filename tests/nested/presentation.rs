use super::Client;
use std::os::fd::AsFd;
use wayland_client::{
    globals::GlobalList,
    protocol::{wl_shm, wl_surface},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::wp::{
    presentation_time::client::{wp_presentation, wp_presentation_feedback as feedback},
    tearing_control::v1::client::{
        wp_tearing_control_manager_v1 as tear, wp_tearing_control_v1 as hint,
    },
};
wayland_client::delegate_noop!(Client: ignore tear::WpTearingControlManagerV1);
wayland_client::delegate_noop!(Client: ignore hint::WpTearingControlV1);
impl Dispatch<wp_presentation::WpPresentation, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wp_presentation::WpPresentation,
        e: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = e {
            assert_eq!(clk_id, 1);
        }
    }
}
impl Dispatch<feedback::WpPresentationFeedback, u32> for Client {
    fn event(
        s: &mut Self,
        _: &feedback::WpPresentationFeedback,
        e: feedback::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            feedback::Event::Discarded => {
                assert_eq!(*id, 0);
                s.discarded = true;
            }
            feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                seq_hi,
                seq_lo,
                flags,
                ..
            } => {
                let stamp = ((u64::from(tv_sec_hi) << 32) + u64::from(tv_sec_lo)) * 1_000_000_000
                    + u64::from(tv_nsec);
                let seq = (u64::from(seq_hi) << 32) + u64::from(seq_lo);
                assert_ne!(*id, 0);
                assert!(stamp > s.presented && refresh > 0 && seq > s.sequence);
                let WEnum::Value(flags) = flags else {
                    panic!("unknown presentation flags")
                };
                if s.drm {
                    assert!(flags.contains(feedback::Kind::Vsync));
                } else {
                    assert!(flags.is_empty());
                }
                println!(
                    "presented seq={seq} ns={stamp} refresh={refresh} flags={}",
                    flags.bits()
                );
                s.presented = stamp;
                s.sequence = seq;
            }
            _ => {}
        }
    }
}

pub fn exercise(
    globals: &GlobalList,
    queue: &mut wayland_client::EventQueue<Client>,
    state: &mut Client,
    surface: &wl_surface::WlSurface,
) -> anyhow::Result<()> {
    let q = queue.handle();
    let presentation: wp_presentation::WpPresentation = globals.bind(&q, 1..=2, ())?;
    let manager: tear::WpTearingControlManagerV1 = globals.bind(&q, 1..=1, ())?;
    let tearing = manager.get_tearing_control(surface, &q, ());
    tearing.set_presentation_hint(hint::PresentationHint::Async);
    let bytes = smithay::reexports::rustix::fs::memfd_create(
        c"frames",
        smithay::reexports::rustix::fs::MemfdFlags::CLOEXEC,
    )?;
    smithay::reexports::rustix::fs::ftruncate(&bytes, 2 * 64 * 64 * 4)?;
    let shm: wl_shm::WlShm = globals.bind(&q, 1..=1, ())?;
    let pool = shm.create_pool(bytes.as_fd(), 2 * 64 * 64 * 4, &q, ());
    let buffers = [0, 64 * 64 * 4]
        .map(|offset| pool.create_buffer(offset, 64, 64, 256, wl_shm::Format::Xrgb8888, &q, ()));
    for i in 0..3 {
        presentation.feedback(surface, &q, i);
        surface.attach(Some(&buffers[(i % 2) as usize]), 0, 0);
        surface.damage_buffer(0, 0, 64, 64);
        surface.commit();
        if i > 0 {
            let prev = state.presented;
            while prev == state.presented {
                queue.blocking_dispatch(state)?;
            }
        }
    }
    assert!(state.discarded);
    println!("superseded commit discarded");
    Ok(())
}
