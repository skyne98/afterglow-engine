use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
#[command(
    name = "console",
    disable_help_subcommand = true,
    disable_version_flag = true
)]
struct ConsoleCli {
    #[command(subcommand)]
    command: ConsoleCommand,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    Connect(ConnectArgs),
    Disconnect,
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
    Net {
        #[command(subcommand)]
        command: NetCommand,
    },
    Stats {
        #[command(subcommand)]
        command: StatsCommand,
    },
    Cvar {
        #[command(subcommand)]
        command: CvarCommand,
    },
    Help(HelpArgs),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct ConnectArgs {
    pub target: String,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ServerCommand {
    Start,
    Stop,
    Status,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum NetCommand {
    Status,
    Stats,
    Links,
    Latency(LatencyArgs),
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct LatencyArgs {
    #[arg(long)]
    pub ms: u32,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum StatsCommand {
    Fps,
    Systems,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum CvarCommand {
    Get { name: String },
    Set { name: String, value: String },
}

#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct HelpArgs {
    pub topic: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsoleParseError {
    pub message: String,
}

pub fn parse_console_command(line: &str) -> Result<ConsoleCommand, ConsoleParseError> {
    let tokens = tokenize_console_line(line)?;
    if tokens.is_empty() {
        return Err(ConsoleParseError {
            message: "empty command".into(),
        });
    }
    let args = std::iter::once("console".to_string()).chain(tokens);
    ConsoleCli::try_parse_from(args)
        .map(|cli| cli.command)
        .map_err(|error| ConsoleParseError {
            message: error.to_string(),
        })
}

pub fn tokenize_console_line(line: &str) -> Result<Vec<String>, ConsoleParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut quoted = false;

    while let Some(ch) = chars.next() {
        match ch {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ch if ch.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if quoted {
        return Err(ConsoleParseError {
            message: "unterminated quote".into(),
        });
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}
