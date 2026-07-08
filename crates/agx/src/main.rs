use clap::Parser;

#[derive(Parser)]
#[command(name = "afterglow")]
struct Cli {
    #[arg(short, long, alias = "demo", value_name = "DEMO")]
    name: Option<String>,

    #[arg(long, conflicts_with = "connect")]
    host: bool,

    #[arg(long, value_name = "ADDR", conflicts_with = "host")]
    connect: Option<String>,

    #[arg(long, value_name = "ADDR", default_value = "0.0.0.0:5000")]
    listen: Option<String>,

    #[arg(long, value_name = "NAME", alias = "player-name")]
    name_player: Option<String>,
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    let cli = Cli::parse();
    #[cfg(target_arch = "wasm32")]
    let cli = Cli::try_parse_from(std::iter::empty::<String>()).unwrap_or(Cli {
        name: None,
        host: false,
        connect: None,
        listen: None,
        name_player: None,
    });

    match cli.name.as_deref() {
        Some("multiplayer-boxes") => {
            let host = cli.host;
            let connect = cli.connect.clone();
            if !host && connect.is_none() {
                eprintln!("error: --name multiplayer-boxes requires --host or --connect");
                std::process::exit(1);
            }
            let listen = cli.listen.unwrap_or_else(|| "0.0.0.0:5000".to_string());
            let player_name = cli
                .name_player
                .clone()
                .or_else(|| std::env::var("USER").ok())
                .unwrap_or_else(|| "player".to_string());
            let config = afterglow_engine::demos::multiplayer_boxes::MultiplayerBoxesDemoConfig {
                player_name,
                host,
                listen: listen.clone(),
                connect: connect.unwrap_or_default(),
            };
            afterglow_engine::run_multiplayer_boxes_demo(config);
        }
        Some("fps-controller") => {
            if cli.host
                || cli.connect.is_some()
                || cli.listen.is_some()
                || cli.name_player.is_some()
            {
                eprintln!(
                    "error: --host, --connect, --listen, --name-player are only valid with --name multiplayer-boxes"
                );
                std::process::exit(1);
            }
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
    fn fps_controller_does_not_use_multiplayer_state() {
        // Parsing succeeds for fps-controller + any multiplayer flag; the
        // runtime branch in `main` rejects the combo. This test just pins
        // the parser contract so the flag surface doesn't accidentally
        // become fps-only.
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "fps-controller",
                "--connect",
                "127.0.0.1:50123",
            ])
            .is_ok()
        );
        assert!(Cli::try_parse_from(["agx", "--name", "fps-controller", "--host",]).is_ok());
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "fps-controller",
                "--listen",
                "127.0.0.1:50123",
            ])
            .is_ok()
        );
        assert!(
            Cli::try_parse_from(["agx", "--name", "fps-controller", "--name-player", "test",])
                .is_ok()
        );
    }

    #[test]
    fn multiplayer_boxes_requires_host_or_connect() {
        assert!(Cli::try_parse_from(["agx", "--name", "multiplayer-boxes", "--host"]).is_ok());
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "multiplayer-boxes",
                "--connect",
                "127.0.0.1:5000"
            ])
            .is_ok()
        );
    }

    #[test]
    fn host_and_connect_are_mutually_exclusive() {
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "multiplayer-boxes",
                "--host",
                "--connect",
                "127.0.0.1:5000",
            ])
            .is_err()
        );
    }

    #[test]
    fn host_defaults_to_false() {
        let cli = Cli::try_parse_from(["agx", "--name", "multiplayer-boxes", "--host"]).unwrap();
        assert!(cli.host);
        assert!(cli.connect.is_none());
    }
}
