use bevy::app::AppExit;
use clap::Parser;

#[derive(Parser)]
#[command(name = "afterglow")]
struct Cli {
    #[arg(short, long)]
    name: Option<String>,
}

fn main() -> AppExit {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let _cli = Cli::parse();

    afterglow_engine::run()
}
