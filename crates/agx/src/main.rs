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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_cli_rejects_multiplayer_flags() {
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "fps-controller",
                "--connect",
                "127.0.0.1:50123",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "fps-controller",
                "--host",
                "127.0.0.1:50123",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "fps-controller",
                "--server",
                "127.0.0.1:50123",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "fps-controller",
                "--listen",
                "127.0.0.1:50123",
            ])
            .is_err()
        );
    }
}
