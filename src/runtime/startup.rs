use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::app::Args;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupOpenState {
    NoPendingStartupOpen,
    WaitingForStartupOpenFiles,
    StartupOpenHandled,
}

pub fn startup_args(base_args: &Args, paths: &[PathBuf]) -> Args {
    if paths.is_empty() {
        return base_args.clone();
    }

    Args {
        show_config: base_args.show_config,
        kak_bin: base_args.kak_bin.clone(),
        kak_args: paths
            .iter()
            .map(|path| path.as_os_str().to_os_string())
            .collect(),
    }
}

pub fn startup_open_state_for_launch(is_macos: bool, kak_args: &[OsString]) -> StartupOpenState {
    if is_macos && kak_args.is_empty() {
        StartupOpenState::WaitingForStartupOpenFiles
    } else {
        StartupOpenState::NoPendingStartupOpen
    }
}

pub fn should_handle_startup_open_with_files(
    state: StartupOpenState,
    has_clients: bool,
    has_session: bool,
) -> bool {
    state == StartupOpenState::WaitingForStartupOpenFiles && !has_clients && !has_session
}

pub fn should_create_fallback_startup_client(
    state: StartupOpenState,
    has_clients: bool,
    has_session: bool,
) -> bool {
    state != StartupOpenState::StartupOpenHandled && !has_clients && !has_session
}

pub fn startup_open_files(kak_args: &[OsString]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut args = kak_args.iter();

    while let Some(arg) = args.next() {
        if arg == "--" {
            files.extend(args.map(PathBuf::from));
            break;
        }

        if matches!(arg.to_str(), Some("-c" | "-C" | "-s" | "-e")) {
            let _ = args.next();
            continue;
        }

        if arg.to_string_lossy().starts_with('-') {
            continue;
        }

        files.push(PathBuf::from(arg));
    }

    files
}

pub fn should_ignore_startup_open_files(
    pending_startup_open_files: &mut Option<Vec<PathBuf>>,
    paths: &[PathBuf],
) -> bool {
    if pending_startup_open_files.as_deref() == Some(paths) {
        *pending_startup_open_files = None;
        true
    } else {
        false
    }
}

pub fn resolve_kakoune_session(kak_args: &[OsString], child_id: u32) -> OsString {
    explicit_kakoune_session(kak_args).unwrap_or_else(|| OsString::from(child_id.to_string()))
}

pub fn explicit_kakoune_session(kak_args: &[OsString]) -> Option<OsString> {
    let mut args = kak_args.iter();
    while let Some(arg) = args.next() {
        if matches!(arg.to_str(), Some("-c" | "-C" | "-s")) {
            return args.next().cloned();
        }
    }

    None
}

pub fn connected_kakoune_args(kak_bin: &str, kakoune_session: &OsStr, paths: &[PathBuf]) -> Args {
    let mut kak_args = vec![OsString::from("-c"), kakoune_session.to_os_string()];
    kak_args.extend(paths.iter().map(|path| path.as_os_str().to_os_string()));
    Args {
        show_config: false,
        kak_bin: kak_bin.to_string(),
        kak_args,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_resolution_uses_child_id_without_explicit_session() {
        assert_eq!(
            resolve_kakoune_session(&[OsString::from("file.txt")], 12345),
            OsString::from("12345")
        );
    }

    #[test]
    fn session_resolution_uses_explicit_server_session() {
        assert_eq!(
            resolve_kakoune_session(
                &[
                    OsString::from("-s"),
                    OsString::from("work"),
                    OsString::from("file.txt"),
                ],
                12345,
            ),
            OsString::from("work")
        );
    }

    #[test]
    fn session_resolution_uses_explicit_client_session() {
        assert_eq!(
            resolve_kakoune_session(
                &[
                    OsString::from("-c"),
                    OsString::from("work"),
                    OsString::from("file.txt"),
                ],
                12345,
            ),
            OsString::from("work")
        );

        assert_eq!(
            resolve_kakoune_session(
                &[
                    OsString::from("-C"),
                    OsString::from("maybe-work"),
                    OsString::from("file.txt"),
                ],
                12345,
            ),
            OsString::from("maybe-work")
        );
    }

    #[test]
    fn startup_open_files_skips_known_option_values() {
        assert_eq!(
            startup_open_files(&[
                OsString::from("-e"),
                OsString::from("echo hi"),
                OsString::from("-s"),
                OsString::from("work"),
                OsString::from("file.txt"),
                OsString::from("--"),
                OsString::from("-literal"),
            ]),
            vec![PathBuf::from("file.txt"), PathBuf::from("-literal")]
        );
    }

    #[test]
    fn startup_open_files_are_ignored_only_once() {
        let mut pending = Some(vec![PathBuf::from("file.txt")]);

        assert!(should_ignore_startup_open_files(
            &mut pending,
            &[PathBuf::from("file.txt")]
        ));
        assert!(!should_ignore_startup_open_files(
            &mut pending,
            &[PathBuf::from("file.txt")]
        ));
    }

    #[test]
    fn startup_open_waits_for_open_files_on_macos_only_for_empty_launch() {
        assert_eq!(
            startup_open_state_for_launch(true, &[]),
            StartupOpenState::WaitingForStartupOpenFiles
        );
        assert_eq!(
            startup_open_state_for_launch(true, &[OsString::from("file.txt")]),
            StartupOpenState::NoPendingStartupOpen
        );
        assert_eq!(
            startup_open_state_for_launch(false, &[]),
            StartupOpenState::NoPendingStartupOpen
        );
    }

    #[test]
    fn fallback_startup_client_is_only_created_when_nothing_else_happened() {
        assert!(should_create_fallback_startup_client(
            StartupOpenState::NoPendingStartupOpen,
            false,
            false
        ));
        assert!(!should_create_fallback_startup_client(
            StartupOpenState::StartupOpenHandled,
            false,
            false
        ));
        assert!(!should_create_fallback_startup_client(
            StartupOpenState::NoPendingStartupOpen,
            true,
            false
        ));
    }
}
