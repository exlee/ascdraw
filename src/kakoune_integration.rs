use std::ffi::OsStr;
use std::path::Path;

use crate::app::{CURSOR_MODE_UI_OPTION, WINDOW_TITLE_UI_OPTION};
use crate::input::{send_keys, send_paste};
use crate::kakoune_messages::{KakouneNotification, StatusStyle};
use crate::runtime::client::{ClientWindow, KakvideHookInstallState};

pub fn bootstrap_client_hooks(client: &mut ClientWindow, notification: &KakouneNotification) -> bool {
    match client.kakvide_hook_install_state {
        KakvideHookInstallState::NotStarted => {
            send_keys(&client.command_tx, &[":".to_string()]);
            client.kakvide_hook_install_state = KakvideHookInstallState::WaitingForCommandPrompt;
            true
        }
        KakvideHookInstallState::WaitingForCommandPrompt => {
            if matches!(
                notification,
                KakouneNotification::DrawStatus {
                    style: StatusStyle::Command,
                    ..
                }
            ) {
                send_paste(
                    &client.command_tx,
                    &format!(
                        " evaluate-commands -draft %{{{}}}",
                        client.kakvide_post_boot_command
                    ),
                );
                send_keys(&client.command_tx, &[String::from("<ret>")]);
                client.kakvide_hook_install_state = KakvideHookInstallState::Installed;
                true
            } else {
                false
            }
        }
        KakvideHookInstallState::Installed => false,
    }
}

pub fn post_boot_command(client_close_socket: Option<&Path>) -> String {
    let mut command = format!(
        "hook global EnterDirectory .* %{{ set-option -add window ui_options \"{0}=%val{{hook_param}} - %val{{client}}\" }}; \
         hook global ModeChange \".*:.*:(.*)\" %{{ set-option -add window ui_options \"{1}=%val{{hook_param_capture_1}}\" }}; \
         set-option -add window ui_options \"{0}=%sh{{pwd}} - %val{{client}}\"; \
         set-option -add window ui_options \"{1}=normal\"",
        WINDOW_TITLE_UI_OPTION, CURSOR_MODE_UI_OPTION
    );
    if let Some(socket) = client_close_socket {
        command.push_str("; ");
        command.push_str(&kakvide_client_close_hook_command(socket));
    }
    command
}

fn kakvide_client_close_hook_command(socket: &Path) -> String {
    format!(
        "hook -once global ClientClose \"^%val{{client}}$\" %{{ nop %sh{{ printf 'KAKVIDE_CLIENT_CLOSE:%s:%s\\n' \"$kak_session\" \"$kak_hook_param\" | nc -U {} }} }}",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_boot_command_tracks_working_directory_without_renaming_client() {
        let command = post_boot_command(None);

        assert!(!command.contains("rename-client"));
        assert!(command.contains("EnterDirectory"));
        assert!(command.contains("ModeChange"));
        assert!(command.contains("%val{hook_param} - %val{client}"));
        assert!(command.contains("%sh{pwd} - %val{client}"));
        assert!(command.contains("%val{hook_param}"));
        assert!(command.contains("%sh{pwd}"));
        assert!(command.contains("kakvide_cursor_mode=%val{hook_param_capture_1}"));
        assert!(command.contains("kakvide_cursor_mode=normal"));
        assert!(!command.contains("buffile"));
    }

    #[cfg(unix)]
    #[test]
    fn post_boot_command_installs_client_close_hook() {
        let command = post_boot_command(Some(Path::new("/tmp/a b.sock")));

        assert!(command.contains("hook -once global ClientClose \"^%val{client}$\""));
        assert!(command.contains("KAKVIDE_CLIENT_CLOSE:%s:%s\\n"));
        assert!(command.contains("nc -U '/tmp/a b.sock'"));
    }
}
