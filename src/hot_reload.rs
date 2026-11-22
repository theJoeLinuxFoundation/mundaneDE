//! Live config reload via inotify on `~/.config/mundane/config.toml`.
//!
//! The settings panel writes to this file; mundaneDE picks the diff up here and applies
//! it to live state — spring params on existing springs, WM gap values, wallpaper
//! crossfade — without a restart. Failure to start the watcher is logged and the
//! compositor continues without it (a restart-only fallback).

use std::path::PathBuf;

use calloop::channel;
use notify::{Event as NotifyEvent, EventKind, RecursiveMode, Watcher};
use smithay::reexports::calloop::{LoopHandle, RegistrationToken};
use tracing::{info, warn};

use crate::animation::SpringKind;
use crate::config::{self, MundaneConfig};
use crate::state::State;

/// Resources kept alive for the lifetime of the compositor: the file watcher and the
/// calloop source token. The watcher itself is `!Send` on some platforms via the
/// Drop guard, so keeping it owned here is intentional.
pub struct HotReload {
    _watcher: notify::RecommendedWatcher,
    /// Calloop source registration. Held so the source isn't unregistered prematurely;
    /// dropping `HotReload` removes it from the loop.
    _token: RegistrationToken,
}

/// Install the watcher on the calloop loop. On any filesystem event that hits the
/// config path the loop callback re-loads the config and applies the diff to [`State`].
pub fn install(handle: &LoopHandle<'static, State>) -> anyhow::Result<Option<HotReload>> {
    let Some(path) = config::config_file_path() else {
        warn!("no XDG_CONFIG_HOME or HOME; skipping config hot-reload");
        return Ok(None);
    };
    // Ensure the parent dir exists so `notify` doesn't fail before the user creates it.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let (tx, rx) = channel::channel::<()>();
    // notify reports raw FS events on its own thread; we just hand a marker through
    // the channel so the calloop callback (running on the main thread) does the actual
    // work against `&mut State`.
    let watched_file = path.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<NotifyEvent>| {
        if let Ok(ev) = res {
            if ev.paths.contains(&watched_file)
                && matches!(
                    ev.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                )
            {
                let _ = tx.send(());
            }
        }
    })?;

    // Watching the *parent directory* is more reliable than the file itself: many
    // editors (and our own atomic-rename writer) replace the file rather than
    // rewriting in place, which makes a file-level inotify watch lose its target.
    let watch_target = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(path.clone());
    if let Err(e) = watcher.watch(&watch_target, RecursiveMode::NonRecursive) {
        warn!("failed to watch {}: {e}", watch_target.display());
        return Ok(None);
    }

    let watched = path.clone();
    let token = handle
        .insert_source(rx, move |event, _, state: &mut State| {
            if let channel::Event::Msg(()) = event {
                apply_reload(state, &watched);
            }
        })
        .map_err(|e| anyhow::anyhow!("hot-reload insert_source failed: {e}"))?;

    info!("hot-reload watching {}", path.display());
    Ok(Some(HotReload {
        _watcher: watcher,
        _token: token,
    }))
}

/// Reload `config.toml` and apply the diff to `state`. Debounced implicitly by the
/// calloop dispatch — multiple FS events between two loop iterations collapse to one.
fn apply_reload(state: &mut State, path: &PathBuf) {
    // Cheap guard: if the file is gone, do nothing (a transient state during atomic
    // rename). The next event will retry.
    if !path.exists() {
        return;
    }
    let new: MundaneConfig = match std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
    {
        Some(config) => config,
        None => {
            warn!("keeping previous config after unreadable edit");
            return;
        }
    };

    // Spring params for position springs.
    let position_params = new.spring.to_spring_params();
    let fade_params = new.wallpaper.to_fade_params();
    for spring in state.animation.springs_mut() {
        match spring.kind {
            SpringKind::Position => spring.params = position_params,
            SpringKind::Fade => spring.params = fade_params,
        }
    }
    state
        .wallpaper
        .retune_fade(fade_params, &mut state.animation);

    // Wallpaper path: if it changed, crossfade.
    if new.wallpaper.path != state.config.wallpaper.path {
        state
            .wallpaper
            .set_wallpaper(new.wallpaper.path.as_deref(), &mut state.animation);
    }
    // Mode update is a swap; no GPU churn.
    if new.wallpaper.mode != state.config.wallpaper.mode {
        state.wallpaper.set_mode(new.wallpaper.mode);
    }

    state.config = new;
    state.generation += 1;

    // Wake the loop so the next iteration redraws with the new settings.
    state.loop_signal.wakeup();

    info!("config reloaded");
}
