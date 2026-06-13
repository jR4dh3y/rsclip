mod actions;
mod app;
mod cli;
mod components;
mod config_reload;
mod dialogs;
mod events;
mod notify;
mod state;
mod style;
mod window;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rsclip=warn".into()),
        )
        .init();

    if let Err(err) = app::run() {
        eprintln!("rsclip: {err:#}");
        std::process::exit(1);
    }
}
