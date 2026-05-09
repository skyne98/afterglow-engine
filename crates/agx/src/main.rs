use clap::Parser;

#[derive(Parser)]
#[command(name = "afterglow")]
struct Cli {
    #[arg(short, long)]
    name: Option<String>,
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    let _cli = Cli::parse();
    #[cfg(target_arch = "wasm32")]
    let _cli = Cli::try_parse_from(std::iter::empty::<String>());

    afterglow_engine::run();
}
