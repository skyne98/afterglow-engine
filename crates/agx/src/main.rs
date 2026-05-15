use clap::Parser;

#[derive(Parser)]
#[command(name = "afterglow")]
struct Cli {
    #[arg(short, long, alias = "demo", value_name = "DEMO")]
    name: Option<String>,
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    let cli = Cli::parse();
    #[cfg(target_arch = "wasm32")]
    let cli = Cli::try_parse_from(std::iter::empty::<String>()).unwrap_or(Cli { name: None });

    match cli.name.as_deref() {
        Some("fps-controller") => {
            afterglow_engine::run_fps_controller_demo();
        }
        _ => {
            afterglow_engine::run();
        }
    }
}
