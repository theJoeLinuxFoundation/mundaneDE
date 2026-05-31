mod animation;
mod backend;
mod config;
mod cursor;
mod focus;
mod render;
mod shell;
mod state;
mod wallpaper;
mod wm;

use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(name = "mundane-de", about = "mundaneDE — one of everything")]
struct Cli {
    /// Use the Winit development backend instead of DRM/KMS
    #[arg(long)]
    winit: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let config = config::MundaneConfig::load(None);

    info!("starting mundaneDE");

    if cli.winit {
        info!("using winit backend");
        backend::winit::run_winit(config)
    } else {
        info!("using DRM/KMS backend");
        backend::drm::run_drm(config)
    }
}
