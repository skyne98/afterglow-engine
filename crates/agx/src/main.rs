use clap::Parser;

#[derive(Parser)]
#[command(name = "afterglow")]
struct Cli {
    #[arg(short, long, alias = "demo", value_name = "DEMO")]
    name: Option<String>,
    #[arg(long, value_name = "ADDR")]
    connect: Option<String>,
    #[arg(long, value_name = "ADDR", conflicts_with = "connect")]
    host: Option<String>,
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    let cli = Cli::parse();
    #[cfg(target_arch = "wasm32")]
    let cli = Cli::try_parse_from(std::iter::empty::<String>()).unwrap_or(Cli {
        name: None,
        connect: None,
        host: None,
    });

    match cli.name.as_deref() {
        Some("fps-controller") => {
            if let Some(host) = cli.host {
                afterglow_engine::run_fps_controller_demo_server(host);
            } else if let Some(connect) = cli.connect {
                afterglow_engine::run_fps_controller_demo_remote(connect);
            } else {
                afterglow_engine::run_fps_controller_demo();
            }
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
    fn fps_cli_connect_targets_remote_server() {
        let cli = Cli::try_parse_from([
            "agx",
            "--name",
            "fps-controller",
            "--connect",
            "127.0.0.1:50123",
        ])
        .unwrap();

        assert_eq!(cli.connect.as_deref(), Some("127.0.0.1:50123"));
        assert_eq!(cli.host, None);
    }

    #[test]
    fn fps_cli_host_targets_server_bind_address() {
        let cli = Cli::try_parse_from([
            "agx",
            "--name",
            "fps-controller",
            "--host",
            "127.0.0.1:50123",
        ])
        .unwrap();

        assert_eq!(cli.host.as_deref(), Some("127.0.0.1:50123"));
        assert_eq!(cli.connect, None);
    }

    #[test]
    fn fps_cli_host_and_connect_conflict() {
        assert!(
            Cli::try_parse_from([
                "agx",
                "--name",
                "fps-controller",
                "--host",
                "127.0.0.1:50123",
                "--connect",
                "127.0.0.1:50123",
            ])
            .is_err()
        );
    }

    #[test]
    fn fps_cli_rejects_removed_server_and_listen_flags() {
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
