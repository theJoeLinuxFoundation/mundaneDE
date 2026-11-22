//! Keep the shell's software-renderer policy out of launched applications.
use gio::prelude::*;
use std::{ffi::OsStr, process::Command, sync::OnceLock};

const COMPILER: &str = "GALLIVM_PERF";
static OWN_DEFAULT: OnceLock<bool> = OnceLock::new();

/// Called at shell entry before starting GPUI, GLib or any worker threads.
pub fn configure() {
    // Gallivm can spend longer optimizing these short-lived UI pipelines than
    // executing them. Mesa includes the flags in its llvmpipe disk-cache key.
    OWN_DEFAULT.get_or_init(|| {
        if std::env::var_os(COMPILER).is_some() {
            return false;
        }
        std::env::set_var(COMPILER, "nopt");
        true
    });
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        // Keep larger temporary driver allocations independently releasable
        // instead of letting glibc raise its mmap threshold after each burst.
        libc::mallopt(libc::M_MMAP_THRESHOLD, 128 * 1024);
    }
}

pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    if OWN_DEFAULT.get() == Some(&true) {
        command.env_remove(COMPILER);
    }
    command
}

pub fn launcher(flags: gio::SubprocessFlags) -> gio::SubprocessLauncher {
    let launcher = gio::SubprocessLauncher::new(flags);
    if OWN_DEFAULT.get() == Some(&true) {
        launcher.unsetenv(COMPILER);
    }
    launcher
}

pub fn application_context() -> gio::AppLaunchContext {
    let context = gio::AppLaunchContext::new();
    if OWN_DEFAULT.get() == Some(&true) {
        context.unsetenv(COMPILER);
    }
    context
}
