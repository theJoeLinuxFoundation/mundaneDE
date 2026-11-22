//! Publish readiness to the GDM session leader and coordinate logind transitions.
use crate::state::State;
use calloop::{generic::Generic, Interest, Mode, PostAction};
use std::{
    cell::RefCell,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

pub fn ready(
    state: &mut State,
    endpoint: &Path,
    sleep: impl FnMut(&mut State, bool) + 'static,
) -> anyhow::Result<()> {
    let mut channel = UnixStream::connect(endpoint)?;
    writeln!(channel, "{}", state.socket_name)?;
    channel.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut ack = [0];
    channel.read_exact(&mut ack)?;
    anyhow::ensure!(ack == *b"1", "session environment was not published");
    state.session_channel = Some(channel.try_clone()?);
    let sleep = Rc::new(RefCell::new(sleep));
    state
        .handle
        .insert_source(
            Generic::new(channel, Interest::READ, Mode::Level),
            move |_, source, state| {
                let mut stream: &UnixStream = source;
                let mut message = [0];
                match stream.read(&mut message)? {
                    1 if message == *b"L" => state.begin_lock(),
                    1 if message == *b"S" => {
                        state.desktop.lock.preparing_sleep = true;
                        state.invalidate_lock_attempt();
                        state.begin_lock();
                        let sleep = sleep.clone();
                        let mut leader = stream.try_clone()?;
                        let start = Instant::now();
                        state
                            .handle
                            .insert_source(
                                calloop::timer::Timer::immediate(),
                                move |_, _, state| {
                                    // An inactive VT cannot page flip. The DRM sleep callback
                                    // also disables active scanouts before releasing the inhibitor.
                                    if !state
                                        .space
                                        .outputs()
                                        .all(|o| state.desktop.lock.covered.contains(o))
                                        && start.elapsed() < Duration::from_secs(2)
                                    {
                                        return calloop::timer::TimeoutAction::ToDuration(
                                            Duration::from_millis(20),
                                        );
                                    }
                                    sleep.borrow_mut()(state, true);
                                    let _ = leader.write_all(b"S");
                                    calloop::timer::TimeoutAction::Drop
                                },
                            )
                            .map_err(|e| std::io::Error::other(e.to_string()))?;
                    }
                    1 if message == *b"W" => {
                        sleep.borrow_mut()(state, false);
                        state.desktop.lock.preparing_sleep = false;
                        state.ensure_locker();
                        state.shell_message(&serde_json::json!({"resume":true}));
                    }
                    _ => {
                        state.loop_signal.stop();
                        return Ok(PostAction::Remove);
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("watch session leader: {e}"))?;
    Ok(())
}
