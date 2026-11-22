//! Local test/apply/revert commands for a running DRM session.
use crate::{config::DisplayConfig, state::State};
use anyhow::{Context, Result};
use calloop::{generic::Generic, Interest, Mode, PostAction};
use std::{collections::BTreeMap, os::unix::net::UnixDatagram, path::PathBuf, time::Duration};

#[derive(Clone, Debug, clap::Subcommand, serde::Deserialize, serde::Serialize)]
pub enum Command {
    /// Try an advertised mode for 15 seconds. Refresh is in millihertz.
    Test {
        name: String,
        width: i32,
        height: i32,
        refresh: i32,
        scale: f64,
        #[arg(default_value_t = 0)]
        rotation: u16,
        #[arg(long,num_args=2,value_names=["X","Y"],allow_hyphen_values=true)]
        position: Vec<i32>,
    },
    /// Keep the current trial for subsequent logins.
    Confirm,
    /// Return immediately to the previous layout.
    Revert,
}
fn runtime() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR").context("missing XDG_RUNTIME_DIR")?,
    ))
}
fn endpoint() -> Result<PathBuf> {
    Ok(runtime()?.join(format!("{}.outputs", std::env::var("WAYLAND_DISPLAY")?)))
}
pub struct SocketPath(PathBuf);
impl Drop for SocketPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn request(cmd: Command) -> Result<()> {
    let path = runtime()?.join(format!("mundane-output-{}", std::process::id()));
    let client = UnixDatagram::bind(&path)?;
    let _guard = SocketPath(path);
    client.set_read_timeout(Some(Duration::from_secs(3)))?;
    client
        .connect(endpoint()?)
        .context("connect to the active DRM session")?;
    // Encode the command in a table so unit variants are also valid TOML.
    let request = toml::to_string(&Message { command: cmd })?;
    client.send(request.as_bytes())?;
    let mut reply = [0; 2048];
    let n = client.recv(&mut reply)?;
    let answer = std::str::from_utf8(&reply[..n])?;
    anyhow::ensure!(answer == "ok", "{answer}");
    Ok(())
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Message {
    command: Command,
}
pub fn listen(
    state: &State,
    mut apply: impl FnMut(&mut State, Command) -> Result<()> + 'static,
) -> Result<SocketPath> {
    let path = endpoint()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let socket = UnixDatagram::bind(&path)?;
    socket.set_nonblocking(true)?;
    state
        .handle
        .insert_source(
            Generic::new(socket, Interest::READ, Mode::Level),
            move |_, socket, state| {
                let mut bytes = [0; 2048];
                let (n, peer) = socket.recv_from(&mut bytes)?;
                let Some(reply) = peer.as_pathname() else {
                    return Ok(PostAction::Continue);
                };
                let command = (|| -> Result<()> {
                    let msg: Message = toml::from_str(std::str::from_utf8(&bytes[..n])?)?;
                    apply(state, msg.command)
                })();
                let response = command
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "ok".into());
                let _ = socket.send_to(response.as_bytes(), reply);
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("register output control: {e}"))?;
    Ok(SocketPath(path))
}
fn saved_path() -> Result<PathBuf> {
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .context("no state directory")?;
    Ok(dir.join("mundane/displays.toml"))
}
pub fn load() -> Result<BTreeMap<String, DisplayConfig>> {
    match std::fs::read_to_string(saved_path()?) {
        Ok(s) => Ok(toml::from_str(&s)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn save(name: &str, config: DisplayConfig) -> Result<()> {
    let path = saved_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut saved = load()?;
    saved.insert(name.into(), config);
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, toml::to_string(&saved)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
