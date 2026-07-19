use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use ascdraw::automation_protocol::{
    AutomationCommand, AutomationRequest, AutomationResponse, KeyModifiers,
};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(about = "Control a running ascdraw instance")]
struct Args {
    #[arg(long, value_name = "PATH")]
    socket: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Ping,
    Key {
        key: String,
        #[arg(long)]
        shift: bool,
        #[arg(long)]
        control: bool,
        #[arg(long)]
        alt: bool,
        #[arg(long = "super")]
        super_key: bool,
        #[arg(long)]
        repeat: bool,
        #[arg(long, default_value_t = 1)]
        count: u32,
    },
    Text {
        text: String,
    },
    Scroll {
        #[arg(allow_hyphen_values = true)]
        x: f32,
        #[arg(allow_hyphen_values = true)]
        y: f32,
        #[arg(long, default_value_t = 1)]
        steps: u32,
    },
    State,
    Metrics {
        #[arg(long)]
        reset: bool,
    },
    Screenshot {
        path: PathBuf,
    },
    Shutdown,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let request = AutomationRequest {
        id: 1,
        command: args.command.into(),
    };
    let response = send_request(&args.socket, &request)?;

    if response.id != request.id {
        bail!(
            "protocol error: response id {} does not match request id {}",
            response.id,
            request.id
        );
    }
    if !response.ok {
        bail!(
            "protocol error: {}",
            response
                .error
                .as_deref()
                .unwrap_or("request failed without an error message")
        );
    }
    if let Some(result) = response.result {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

impl From<Command> for AutomationCommand {
    fn from(command: Command) -> Self {
        match command {
            Command::Ping => Self::Ping,
            Command::Key {
                key,
                shift,
                control,
                alt,
                super_key,
                repeat,
                count,
            } => Self::Key {
                key,
                modifiers: KeyModifiers {
                    shift,
                    control,
                    alt,
                    super_key,
                },
                repeat,
                count,
            },
            Command::Text { text } => Self::Text { text },
            Command::Scroll { x, y, steps } => Self::Scroll { x, y, steps },
            Command::State => Self::State,
            Command::Metrics { reset } => Self::Metrics { reset },
            Command::Screenshot { path } => Self::Screenshot { path },
            Command::Shutdown => Self::Shutdown,
        }
    }
}

#[cfg(unix)]
fn send_request(socket: &PathBuf, request: &AutomationRequest) -> Result<AutomationResponse> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    use anyhow::Context;

    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("failed to connect to socket {}", socket.display()))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .context("failed to set response timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .context("failed to set request timeout")?;
    serde_json::to_writer(&mut stream, request).context("failed to encode request")?;
    stream.write_all(b"\n").context("failed to send request")?;
    stream.flush().context("failed to send request")?;

    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .context("failed to read response")?;
    if line.is_empty() {
        bail!("protocol error: server closed the connection without a response");
    }

    serde_json::from_str(&line).context("protocol error: invalid response JSON")
}

#[cfg(not(unix))]
fn send_request(_socket: &PathBuf, _request: &AutomationRequest) -> Result<AutomationResponse> {
    bail!("ascdrawctl is unsupported on this platform: Unix domain sockets are required")
}
