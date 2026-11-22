mod animation;
mod backend;
mod config;
mod cursor;
mod focus;
mod hot_reload;
mod outputs;
mod render;
mod shell;
mod state;
mod tearing;
mod wallpaper;
mod wm;
mod x11;

use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(name = "mundane-de", about = "mundaneDE — one of everything")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Use the Winit development backend instead of DRM/KMS
    #[arg(long)]
    winit: bool,
    /// Private readiness socket of the GDM session launcher.
    #[arg(long, hide = true, conflicts_with_all = ["winit", "x11_display"])]
    session_notify: Option<std::path::PathBuf>,
    /// Ask a running mundaneDE session to start Xwayland and print its DISPLAY.
    #[arg(long, conflicts_with = "winit")]
    x11_display: bool,
}

#[derive(clap::Subcommand)]
enum Command {
    #[command(hide = true)]
    WatchToplevels,
    /// Test, confirm or revert a DRM output configuration.
    Output {
        #[command(subcommand)]
        action: backend::output_control::Command,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if matches!(cli.command,Some(Command::WatchToplevels)){return shell::toplevels::watch();}
    if let Some(Command::Output { action }) = cli.command {
        return backend::output_control::request(action);
    }
    if cli.x11_display {
        return x11::print_display();
    }
    let config = config::MundaneConfig::load(None);

    info!("starting mundaneDE");

    if cli.winit {
        info!("using winit backend");
        backend::winit::run_winit(config)
    } else {
        info!("using DRM/KMS backend");
        backend::drm::run_drm(config, cli.session_notify.as_deref())
    }
}
