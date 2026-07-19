use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ascdraw::automation_protocol::{AutomationRequest, AutomationResponse};
use winit::event_loop::EventLoopProxy;

use crate::app::{AppEvent, AutomationEnvelope};

pub struct AutomationServer {
    path: PathBuf,
}

impl AutomationServer {
    pub fn start(path: PathBuf, proxy: EventLoopProxy<AppEvent>) -> Result<Self> {
        prepare_socket_path(&path)?;
        let listener = std::os::unix::net::UnixListener::bind(&path)
            .with_context(|| format!("failed to bind automation socket {}", path.display()))?;
        set_private_permissions(&path)?;
        thread::Builder::new()
            .name("ascdraw-automation".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => {
                            let proxy = proxy.clone();
                            if let Err(error) = thread::Builder::new()
                                .name("ascdraw-automation-client".to_owned())
                                .spawn(move || handle_connection(stream, &proxy))
                            {
                                crate::diagnostics::log_error(format!(
                                    "automation connection thread failed: {error}"
                                ));
                            }
                        }
                        Err(error) => {
                            crate::diagnostics::log_error(format!(
                                "automation socket failed: {error}"
                            ));
                            break;
                        }
                    }
                }
            })
            .context("failed to start automation server")?;
        Ok(Self { path })
    }

    pub fn cleanup(&self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn prepare_socket_path(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        bail!(
            "automation socket parent does not exist: {}",
            parent.display()
        );
    }
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            bail!("automation socket is already in use: {}", path.display());
        }
        use std::os::unix::fs::FileTypeExt;

        if !fs::symlink_metadata(path)?.file_type().is_socket() {
            bail!("automation socket path is not a socket: {}", path.display());
        }
        fs::remove_file(path)
            .with_context(|| format!("failed to remove stale socket {}", path.display()))?;
    }
    Ok(())
}

fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure automation socket {}", path.display()))
}

fn handle_connection(stream: std::os::unix::net::UnixStream, proxy: &EventLoopProxy<AppEvent>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let reader_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(error) => {
            crate::diagnostics::log_error(format!("automation connection failed: {error}"));
            return;
        }
    };
    let mut writer = stream;
    for line in BufReader::new(reader_stream).lines() {
        let response = match line {
            Ok(line) => dispatch_line(&line, proxy),
            Err(error) => AutomationResponse::error(0, format!("failed to read request: {error}")),
        };
        if serde_json::to_writer(&mut writer, &response).is_err()
            || writer.write_all(b"\n").is_err()
            || writer.flush().is_err()
        {
            return;
        }
    }
}

fn dispatch_line(line: &str, proxy: &EventLoopProxy<AppEvent>) -> AutomationResponse {
    let request = match serde_json::from_str::<AutomationRequest>(line) {
        Ok(request) => request,
        Err(error) => return AutomationResponse::error(0, format!("invalid request: {error}")),
    };
    let id = request.id;
    let (response, receiver) = mpsc::channel();
    if proxy
        .send_event(AppEvent::Automation(AutomationEnvelope {
            request,
            response,
        }))
        .is_err()
    {
        return AutomationResponse::error(id, "application event loop is unavailable");
    }
    receiver
        .recv()
        .unwrap_or_else(|_| AutomationResponse::error(id, "application stopped before responding"))
}

#[cfg(test)]
mod tests {
    use super::prepare_socket_path;

    #[test]
    fn socket_parent_must_exist() {
        let path = std::env::temp_dir()
            .join("ascdraw-missing-automation-parent")
            .join("control.sock");
        assert!(prepare_socket_path(&path).is_err());
    }
}
