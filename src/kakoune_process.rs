use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result};
use winit::event_loop::EventLoopProxy;

use crate::app::{AppEvent, Args};
use crate::kakoune_messages::parse_notification;

pub fn spawn_kakoune(args: &Args, proxy: EventLoopProxy<AppEvent>) -> Result<Child> {
    let mut command = build_kakoune_command(args);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {}", args.kak_bin))?;

    let stdout = child.stdout.take().context("missing kakoune stdout pipe")?;
    let stderr = child.stderr.take().context("missing kakoune stderr pipe")?;

    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => match parse_notification(&line) {
                    Ok(notification) => {
                        let _ = proxy.send_event(AppEvent::Rpc(Box::new(notification)));
                    }
                    Err(error) => eprintln!("json ui parse error: {error:#}\nline: {line}"),
                },
                Err(error) => {
                    eprintln!("stdout read error: {error:#}");
                    break;
                }
            }
        }
        let _ = proxy.send_event(AppEvent::KakouneExited);
    });

    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(line) => eprintln!("kak stderr: {line}"),
                Err(error) => {
                    eprintln!("stderr read error: {error:#}");
                    break;
                }
            }
        }
    });

    Ok(child)
}

fn build_kakoune_command(args: &Args) -> Command {
    let mut command = Command::new(&args.kak_bin);
    command.arg("-ui").arg("json");
    command.args(&args.kak_args);
    command
}

pub fn spawn_stdin_writer(child: &mut Child) -> Result<Sender<String>> {
    let stdin = child.stdin.take().context("missing kakoune stdin pipe")?;
    let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();

    thread::spawn(move || {
        let mut stdin = stdin;
        while let Ok(line) = rx.recv() {
            if stdin.write_all(line.as_bytes()).is_err() {
                break;
            }
            if stdin.write_all(b"\n").is_err() {
                break;
            }
            if stdin.flush().is_err() {
                break;
            }
        }
    });

    Ok(tx)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::build_kakoune_command;
    use crate::app::Args;

    #[test]
    fn build_kakoune_command_includes_json_ui_before_forwarded_args() {
        let args = Args {
            kak_bin: "kak".to_string(),
            kak_args: vec![
                OsString::from("-d"),
                OsString::from("-e"),
                OsString::from("echo hi"),
                OsString::from("file.txt"),
            ],
        };

        let command = build_kakoune_command(&args);
        let actual_args: Vec<_> = command.get_args().map(OsString::from).collect();

        assert_eq!(
            actual_args,
            vec![
                OsString::from("-ui"),
                OsString::from("json"),
                OsString::from("-d"),
                OsString::from("-e"),
                OsString::from("echo hi"),
                OsString::from("file.txt"),
            ]
        );
    }
}
