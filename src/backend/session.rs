//! Publish readiness only to the GDM session leader, then watch for its exit.
use crate::state::State;
use calloop::{generic::Generic, Interest, Mode, PostAction};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

pub fn ready(
    state: &mut State,
    endpoint: &Path,
    mut sleep: impl FnMut(&mut State, bool) + 'static,
) -> anyhow::Result<()> {
    let mut channel = UnixStream::connect(endpoint)?;
    writeln!(channel, "{}", state.socket_name)?;
    channel.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut ack = [0];
    channel.read_exact(&mut ack)?;
    anyhow::ensure!(ack == *b"1", "session environment was not published");
    state.session_channel = Some(channel.try_clone()?);
    state
        .handle
        .insert_source(
            Generic::new(channel, Interest::READ, Mode::Level),
            move |_, source, state| {
                let mut stream: &UnixStream = source;
                let mut message = [0];
                match stream.read(&mut message)? {
                    1 if message == *b"S" => {
                        sleep(state, true);
                        stream.write_all(b"S")?;
                    }
                    1 if message == *b"W" => sleep(state, false),
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
