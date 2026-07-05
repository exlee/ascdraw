use std::borrow::Cow;
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(unix)]
use std::os::unix::net::UnixListener;

use anyhow::{Context, Result};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::app::{AppEvent, Args, WINDOW_TITLE_UI_OPTION};
use crate::diagnostics::log_error;
use crate::kakoune_messages::parse_notification;

pub fn spawn_kakoune(
    args: &Args,
    proxy: EventLoopProxy<AppEvent>,
    window_id: WindowId,
    client_id: &str,
    client_close_socket: Option<&Path>,
) -> Result<Child> {
    let mut command = build_kakoune_command(args, client_id, client_close_socket);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start {}", args.kak_bin))?;

    let stdout = child.stdout.take().context("missing kakoune stdout pipe")?;
    let stderr = child.stderr.take().context("missing kakoune stderr pipe")?;

    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut raw_line = Vec::new();
        loop {
            raw_line.clear();
            match reader.read_until(b'\n', &mut raw_line) {
                Ok(0) => break,
                Ok(_) => {
                    let line = decode_json_ui_line(&raw_line);
                    if let Some(error) = line.utf8_error {
                        log_error(format!(
                            "json ui stdout contained invalid utf-8: {error}; decoded {} bytes lossily",
                            line.byte_len
                        ));
                    }
                    match parse_notification(&line.text) {
                        Ok(notification) => {
                            let _ =
                                proxy.send_event(AppEvent::Rpc(window_id, Box::new(notification)));
                        }
                        Err(error) => log_error(format!(
                            "json ui parse error: {error:#}\nline: {}",
                            line.text
                        )),
                    }
                }
                Err(error) => {
                    log_error(format!("stdout read error: {error:#}"));
                    break;
                }
            }
        }
        let _ = proxy.send_event(AppEvent::KakouneExited(window_id));
    });

    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(line) => log_error(format!("kak stderr: {line}")),
                Err(error) => {
                    log_error(format!("stderr read error: {error:#}"));
                    break;
                }
            }
        }
    });

    Ok(child)
}

#[cfg(unix)]
pub struct ClientCloseListener {
    path: PathBuf,
}

#[cfg(unix)]
impl ClientCloseListener {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
impl Drop for ClientCloseListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
pub fn spawn_client_close_listener(proxy: EventLoopProxy<AppEvent>) -> Result<ClientCloseListener> {
    let path = env::temp_dir().join(format!("kakvide-client-close-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("failed to bind client close socket {}", path.display()))?;

    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let mut message = String::new();
                    match stream.read_to_string(&mut message) {
                        Ok(_) => {
                            if let Some((session, client_id)) =
                                parse_client_close_message(message.trim())
                            {
                                let _ =
                                    proxy.send_event(AppEvent::ClientClosed { session, client_id });
                            }
                        }
                        Err(error) => {
                            log_error(format!("client close socket read error: {error:#}"))
                        }
                    }
                }
                Err(error) => {
                    log_error(format!("client close socket accept error: {error:#}"));
                    break;
                }
            }
        }
    });

    Ok(ClientCloseListener { path })
}

fn parse_client_close_message(message: &str) -> Option<(OsString, String)> {
    let rest = message.strip_prefix("KAKVIDE_CLIENT_CLOSE:")?;
    let (session, client_id) = rest.split_once(':')?;
    if session.is_empty() || client_id.is_empty() {
        return None;
    }

    Some((OsString::from(session), client_id.to_string()))
}

struct JsonUiLine<'a> {
    text: Cow<'a, str>,
    utf8_error: Option<std::str::Utf8Error>,
    byte_len: usize,
}

fn decode_json_ui_line(raw_line: &[u8]) -> JsonUiLine<'_> {
    let line = trim_line_ending(raw_line);
    match std::str::from_utf8(line) {
        Ok(text) => JsonUiLine {
            text: Cow::Borrowed(text),
            utf8_error: None,
            byte_len: line.len(),
        },
        Err(error) => JsonUiLine {
            text: String::from_utf8_lossy(line),
            utf8_error: Some(error),
            byte_len: line.len(),
        },
    }
}

fn trim_line_ending(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn build_kakoune_command(
    args: &Args,
    client_id: &str,
    client_close_socket: Option<&Path>,
) -> Command {
    platform_kakoune_command(args, client_id, client_close_socket)
}

pub fn build_kakoune_help_command(kak_bin: &OsStr) -> Command {
    platform_kakoune_help_command(kak_bin)
}

pub fn list_kakoune_sessions(kak_bin: &str) -> Result<Vec<OsString>> {
    let output = platform_kakoune_list_sessions_command(OsStr::new(kak_bin))
        .output()
        .with_context(|| format!("failed to run {kak_bin} -l"))?;

    if !output.status.success() {
        anyhow::bail!("{kak_bin} -l exited with {}", output.status);
    }

    Ok(parse_kakoune_sessions(&output.stdout))
}

fn parse_kakoune_sessions(output: &[u8]) -> Vec<OsString> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(OsString::from)
        .collect()
}

fn append_kakoune_json_ui_args(
    command: &mut Command,
    args: &Args,
    client_id: &str,
    client_close_socket: Option<&Path>,
) {
    command
        .arg("-ui")
        .arg("json")
        .arg("-e")
        .arg(kakvide_client_init_command(client_id, client_close_socket))
        .args(&args.kak_args);
}

fn kakvide_client_init_command(client_id: &str, client_close_socket: Option<&Path>) -> String {
    let mut command = format!(
        "rename-client {client_id}; \
         hook global EnterDirectory .* %{{ set-option -add window ui_options \"{0}=%val{{hook_param}} - %val{{client}}\" }}; \
         set-option -add window ui_options \"{0}=%sh{{pwd}} - %val{{client}}\"",
        WINDOW_TITLE_UI_OPTION
    );
    if let Some(socket) = client_close_socket {
        command.push_str("; ");
        command.push_str(&kakvide_client_close_hook_command(client_id, socket));
    }
    command
}

fn kakvide_client_close_hook_command(client_id: &str, socket: &Path) -> String {
    format!(
        "hook -once global ClientClose ^{client_id}$ %{{ nop %sh{{ printf 'KAKVIDE_CLIENT_CLOSE:%s:%s\\n' \"$kak_session\" \"$kak_hook_param\" | nc -U {} }} }}",
        shell_quote(socket.as_os_str())
    )
}

#[cfg(unix)]
fn shell_quote(value: &OsStr) -> String {
    let mut quoted = String::from("'");
    for part in value.to_string_lossy().split('\'') {
        if quoted.len() > 1 {
            quoted.push_str("'\\''");
        }
        quoted.push_str(part);
    }
    quoted.push('\'');
    quoted
}

#[cfg(not(unix))]
fn shell_quote(value: &OsStr) -> String {
    format!("{:?}", value.to_string_lossy())
}

#[cfg(unix)]
fn platform_kakoune_command(
    args: &Args,
    client_id: &str,
    client_close_socket: Option<&Path>,
) -> Command {
    if let Some(shell) = user_shell() {
        let mut command = shell_command(shell, OsStr::new(&args.kak_bin));
        append_kakoune_json_ui_args(&mut command, args, client_id, client_close_socket);
        command
    } else {
        let (program, constrained_path) = constrained_app_program(OsStr::new(&args.kak_bin));
        let mut command = Command::new(program);
        apply_constrained_app_path(&mut command, constrained_path);
        append_kakoune_json_ui_args(&mut command, args, client_id, client_close_socket);
        command
    }
}

#[cfg(unix)]
fn platform_kakoune_help_command(kak_bin: &OsStr) -> Command {
    if let Some(shell) = user_shell() {
        let mut command = shell_command(shell, kak_bin);
        command.arg("--help");
        command
    } else {
        let (program, constrained_path) = constrained_app_program(kak_bin);
        let mut command = Command::new(program);
        apply_constrained_app_path(&mut command, constrained_path);
        command.arg("--help");
        command
    }
}

#[cfg(unix)]
fn platform_kakoune_list_sessions_command(kak_bin: &OsStr) -> Command {
    if let Some(shell) = user_shell() {
        let mut command = shell_command(shell, kak_bin);
        command.arg("-l");
        command
    } else {
        let (program, constrained_path) = constrained_app_program(kak_bin);
        let mut command = Command::new(program);
        apply_constrained_app_path(&mut command, constrained_path);
        command.arg("-l");
        command
    }
}

#[cfg(windows)]
fn platform_kakoune_command(
    args: &Args,
    client_id: &str,
    client_close_socket: Option<&Path>,
) -> Command {
    let mut command = Command::new(&args.kak_bin);
    append_kakoune_json_ui_args(&mut command, args, client_id, client_close_socket);
    command
}

#[cfg(windows)]
fn platform_kakoune_help_command(kak_bin: &OsStr) -> Command {
    let mut command = Command::new(kak_bin);
    command.arg("--help");
    command
}

#[cfg(windows)]
fn platform_kakoune_list_sessions_command(kak_bin: &OsStr) -> Command {
    let mut command = Command::new(kak_bin);
    command.arg("-l");
    command
}

#[cfg(unix)]
fn user_shell() -> Option<OsString> {
    env::var_os("SHELL").filter(|shell| !shell.is_empty())
}

#[cfg(unix)]
fn shell_command(shell: OsString, kak_bin: &OsStr) -> Command {
    let mut command = Command::new(shell);
    command
        .arg("-lc")
        .arg("exec \"$@\"")
        .arg("kakvide-kak")
        .arg(kak_bin);
    command
}

#[cfg(unix)]
fn constrained_app_program(program: &OsStr) -> (OsString, Option<OsString>) {
    let path = constrained_app_path(env::var_os("PATH"));
    let resolved = resolve_from_path(program, &path).unwrap_or_else(|| program.to_os_string());
    (resolved, Some(path))
}

#[cfg(unix)]
fn apply_constrained_app_path(command: &mut Command, constrained_path: Option<OsString>) {
    let Some(path) = constrained_path else {
        return;
    };

    command.env("PATH", path);
}

#[cfg(unix)]
fn constrained_app_path(path: Option<OsString>) -> OsString {
    let mut paths = vec![
        OsString::from("/usr/local/bin/"),
        OsString::from("/opt/homebrew/bin"),
        OsString::from("~/.local/bin"),
    ];
    if let Some(path) = path
        && !path.is_empty()
    {
        paths.push(path);
    }

    join_unix_paths(paths)
}

#[cfg(unix)]
fn join_unix_paths(paths: Vec<OsString>) -> OsString {
    let mut joined = OsString::new();
    for (index, path) in paths.into_iter().enumerate() {
        if index > 0 {
            joined.push(":");
        }
        joined.push(path);
    }
    joined
}

#[cfg(unix)]
fn resolve_from_path(program: &OsStr, path: &OsStr) -> Option<OsString> {
    if program.as_bytes().contains(&b'/') {
        return None;
    }

    for dir in split_unix_path(path) {
        let candidate = expand_home_dir(&dir).join(program);
        if candidate.is_file() {
            return Some(candidate.into_os_string());
        }
    }
    None
}

#[cfg(unix)]
fn split_unix_path(path: &OsStr) -> Vec<OsString> {
    path.as_bytes()
        .split(|byte| *byte == b':')
        .map(|part| OsString::from_vec(part.to_vec()))
        .collect()
}

#[cfg(unix)]
fn expand_home_dir(path: &OsStr) -> PathBuf {
    expand_home_dir_with_home(path, env::var_os("HOME"))
}

#[cfg(unix)]
fn expand_home_dir_with_home(path: &OsStr, home: Option<OsString>) -> PathBuf {
    if path == "~"
        && let Some(home) = home.as_ref()
    {
        return PathBuf::from(home);
    }

    if let Some(rest) = path.to_str().and_then(|path| path.strip_prefix("~/"))
        && let Some(home) = home
    {
        return PathBuf::from(home).join(rest);
    }

    PathBuf::from(path)
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
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::Path;

    use crate::kakoune_messages::KakouneNotification;

    use super::build_kakoune_command;
    #[cfg(windows)]
    use crate::app::Args;

    #[test]
    fn json_ui_line_decoder_accepts_valid_utf8_line() {
        let decoded = super::decode_json_ui_line(b"{\"jsonrpc\":\"2.0\"}\n");

        assert_eq!(decoded.text, "{\"jsonrpc\":\"2.0\"}");
        assert!(decoded.utf8_error.is_none());
        assert_eq!(decoded.byte_len, 17);
    }

    #[test]
    fn json_ui_line_decoder_trims_crlf() {
        let decoded = super::decode_json_ui_line(b"{\"jsonrpc\":\"2.0\"}\r\n");

        assert_eq!(decoded.text, "{\"jsonrpc\":\"2.0\"}");
        assert!(decoded.utf8_error.is_none());
        assert_eq!(decoded.byte_len, 17);
    }

    #[test]
    fn json_ui_line_decoder_replaces_invalid_utf8_in_atom_contents() {
        let mut raw = br#"{"jsonrpc":"2.0","method":"draw","params":[[[{"face":{"fg":"default","bg":"default","underline":"default","attributes":[]},"contents":"a"#.to_vec();
        raw.push(0xff);
        raw.extend_from_slice(
            br#"b"}]],{"line":0,"column":0},{"fg":"default","bg":"default","underline":"default","attributes":[]},{"fg":"blue","bg":"default","underline":"default","attributes":[]},0]}"#,
        );
        raw.push(b'\n');

        let decoded = super::decode_json_ui_line(&raw);

        assert!(decoded.utf8_error.is_some());
        match crate::kakoune_messages::parse_notification(&decoded.text).unwrap() {
            KakouneNotification::Draw { lines, .. } => {
                assert_eq!(lines[0][0].contents, "a\u{fffd}b");
            }
            other => panic!("unexpected notification: {other:?}"),
        }
    }

    #[test]
    fn client_init_command_renames_client_and_tracks_working_directory() {
        let command = super::kakvide_client_init_command("kakvide-123-0", None);

        assert!(command.contains("rename-client kakvide-123-0"));
        assert!(command.contains("EnterDirectory"));
        assert!(command.contains("%val{hook_param} - %val{client}"));
        assert!(command.contains("%sh{pwd} - %val{client}"));
        assert!(command.contains("%val{hook_param}"));
        assert!(command.contains("%sh{pwd}"));
        assert!(!command.contains("buffile"));
    }

    #[cfg(unix)]
    #[test]
    fn client_init_command_installs_client_close_hook() {
        let command =
            super::kakvide_client_init_command("kakvide-123-0", Some(Path::new("/tmp/a b.sock")));

        assert!(command.contains("hook -once global ClientClose ^kakvide-123-0$"));
        assert!(command.contains("KAKVIDE_CLIENT_CLOSE:%s:%s\\n"));
        assert!(command.contains("nc -U '/tmp/a b.sock'"));
    }

    #[test]
    fn client_close_message_parser_extracts_session_and_client() {
        assert_eq!(
            super::parse_client_close_message("KAKVIDE_CLIENT_CLOSE:work:kakvide-123-0"),
            Some((OsString::from("work"), "kakvide-123-0".to_string()))
        );
    }

    #[test]
    fn session_list_parser_trims_blank_lines() {
        assert_eq!(
            super::parse_kakoune_sessions(b"\nmain\n  scratch  \n\n"),
            vec![OsString::from("main"), OsString::from("scratch")]
        );
    }

    #[cfg(windows)]
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

        let command = build_kakoune_command(&args, "client0", None);
        let actual_args: Vec<_> = command.get_args().map(OsString::from).collect();

        assert_eq!(
            actual_args,
            vec![
                OsString::from("-ui"),
                OsString::from("json"),
                OsString::from("-e"),
                OsString::from(super::kakvide_client_init_command("client0", None)),
                OsString::from("-d"),
                OsString::from("-e"),
                OsString::from("echo hi"),
                OsString::from("file.txt"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_command_runs_kak_through_shell_lc() {
        let args = crate::app::Args {
            kak_bin: "kak".to_string(),
            kak_args: vec![OsString::from("file.txt")],
        };
        let command = build_kakoune_command(&args, "client0", None);
        let actual_args: Vec<_> = command.get_args().map(OsString::from).collect();

        assert_eq!(
            actual_args,
            vec![
                OsString::from("-lc"),
                OsString::from("exec \"$@\""),
                OsString::from("kakvide-kak"),
                OsString::from("kak"),
                OsString::from("-ui"),
                OsString::from("json"),
                OsString::from("-e"),
                OsString::from(super::kakvide_client_init_command("client0", None)),
                OsString::from("file.txt"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_shell_help_command_runs_kak_through_shell_lc() {
        let mut command = super::shell_command(OsString::from("/bin/sh"), OsStr::new("kak"));
        command.arg("--help");
        let actual_args: Vec<_> = command.get_args().map(OsString::from).collect();

        assert_eq!(
            actual_args,
            vec![
                OsString::from("-lc"),
                OsString::from("exec \"$@\""),
                OsString::from("kakvide-kak"),
                OsString::from("kak"),
                OsString::from("--help"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn constrained_app_path_prepends_search_paths() {
        assert_eq!(
            super::constrained_app_path(Some(OsString::from("/usr/bin:/bin"))),
            OsString::from("/usr/local/bin/:/opt/homebrew/bin:~/.local/bin:/usr/bin:/bin")
        );
    }

    #[cfg(unix)]
    #[test]
    fn constrained_app_path_uses_only_search_paths_without_existing_path() {
        assert_eq!(
            super::constrained_app_path(None),
            OsString::from("/usr/local/bin/:/opt/homebrew/bin:~/.local/bin")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolves_program_from_constrained_path_with_expanded_home() {
        let base =
            std::env::temp_dir().join(format!("kakvide-kak-bin-test-{}", std::process::id()));
        let home_bin = base.join("home/.local/bin");
        fs::create_dir_all(&home_bin).expect("test bin dir should be created");
        let kak = home_bin.join("kak");
        fs::write(&kak, "").expect("test kak file should be created");

        assert_eq!(
            super::expand_home_dir_with_home(
                OsStr::new("~/.local/bin"),
                Some(base.join("home").into_os_string())
            ),
            home_bin
        );

        assert_eq!(
            super::resolve_from_path(OsStr::new("kak"), home_bin.as_os_str()),
            Some(kak.into_os_string())
        );
        let _ = fs::remove_dir_all(base);
    }
}
