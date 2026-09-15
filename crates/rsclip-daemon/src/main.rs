mod commands;
mod favicons;
mod watcher;

use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rsclipd=info".into()),
        )
        .init();

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    commands::run(&args)
}
