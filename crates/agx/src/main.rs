use clap::Parser;

#[derive(Parser)]
#[command(name = "afterglow")]
struct Cli {
    #[arg(short, long)]
    name: Option<String>,
}

fn main() -> bevy::app::AppExit {
    let _cli = Cli::parse();
    afterglow_engine::run()
}
