use std::backtrace::Backtrace;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::path::PathBuf;
use std::process;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
enum LogPlatform {
    Macos,
    Linux,
    Windows,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LocalDateTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl LocalDateTime {
    fn date_stamp(self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    fn timestamp(self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

pub fn init() -> io::Result<PathBuf> {
    let path = current_log_path();
    init_at(path)
}

fn init_at(path: PathBuf) -> io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let _ = LOG_FILE.set(Mutex::new(file));
    write_entry("diagnostics initialized");
    Ok(path)
}

pub fn install_panic_hook() {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        log_panic(info);
        previous_hook(info);
    }));
}

pub fn log_error(message: impl AsRef<str>) {
    let message = message.as_ref();
    write_entry(message);
    eprintln!("{message}");
}

pub fn log_info(message: impl AsRef<str>) {
    let message = message.as_ref();
    write_entry(message);
    eprintln!("{message}");
}

fn log_panic(info: &PanicHookInfo<'_>) {
    let thread = thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");

    let location = info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "<unknown>".to_string());
    let backtrace = Backtrace::force_capture();

    write_entry(format!(
        "panic: process={} thread={thread_name:?} thread_id={:?} location={location} payload={payload}\nbacktrace:\n{backtrace}",
        process::id(),
        thread.id()
    ));
}

fn write_entry(message: impl AsRef<str>) {
    let Some(file) = LOG_FILE.get() else {
        return;
    };
    let Ok(mut file) = file.lock() else {
        return;
    };

    let _ = writeln!(file, "[{}] {}", local_now().timestamp(), message.as_ref());
    let _ = file.flush();
}

fn current_log_path() -> PathBuf {
    let date = local_now().date_stamp();
    log_path_for_platform(
        current_platform(),
        |name| env::var_os(name),
        env::temp_dir(),
        &date,
    )
}

#[cfg(target_os = "macos")]
fn current_platform() -> LogPlatform {
    LogPlatform::Macos
}

#[cfg(target_os = "linux")]
fn current_platform() -> LogPlatform {
    LogPlatform::Linux
}

#[cfg(target_os = "windows")]
fn current_platform() -> LogPlatform {
    LogPlatform::Windows
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn current_platform() -> LogPlatform {
    LogPlatform::Other
}

fn log_path_for_platform(
    platform: LogPlatform,
    env_var: impl Fn(&str) -> Option<OsString>,
    temp_dir: PathBuf,
    date: &str,
) -> PathBuf {
    let filename = format!("ascdraw.{date}.log");
    let directory = match platform {
        LogPlatform::Macos => env_var("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Logs").join("ascdraw")),
        LogPlatform::Linux => env_var("XDG_STATE_HOME")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .map(|path| path.join("ascdraw"))
            .or_else(|| {
                env_var("HOME")
                    .filter(|home| !home.is_empty())
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("state").join("ascdraw"))
            }),
        LogPlatform::Windows => env_var("LOCALAPPDATA")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .map(|path| path.join("ascdraw")),
        LogPlatform::Other => None,
    };

    directory
        .unwrap_or_else(|| temp_dir.join("ascdraw"))
        .join(filename)
}

fn local_now() -> LocalDateTime {
    local_datetime(SystemTime::now())
}

fn local_datetime(time: SystemTime) -> LocalDateTime {
    let duration = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    local_datetime_from_unix_seconds(duration.as_secs() as i64)
}

#[cfg(unix)]
fn local_datetime_from_unix_seconds(seconds: i64) -> LocalDateTime {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "haiku",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "solaris",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos"
        ))]
        tm_gmtoff: i64,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "haiku",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "solaris",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos"
        ))]
        tm_zone: *const std::ffi::c_char,
    }

    unsafe extern "C" {
        fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
    }

    let mut result = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 1,
        tm_mon: 0,
        tm_year: 70,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "haiku",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "solaris",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos"
        ))]
        tm_gmtoff: 0,
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "haiku",
            target_os = "illumos",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "solaris",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos"
        ))]
        tm_zone: std::ptr::null(),
    };

    let local = unsafe { localtime_r(&seconds, &mut result) };
    if local.is_null() {
        return utc_datetime_from_unix_seconds(seconds);
    }

    LocalDateTime {
        year: result.tm_year + 1900,
        month: (result.tm_mon + 1).max(1) as u32,
        day: result.tm_mday.max(1) as u32,
        hour: result.tm_hour.max(0) as u32,
        minute: result.tm_min.max(0) as u32,
        second: result.tm_sec.max(0) as u32,
    }
}

#[cfg(windows)]
fn local_datetime_from_unix_seconds(_seconds: i64) -> LocalDateTime {
    #[repr(C)]
    struct SystemTime {
        w_year: u16,
        w_month: u16,
        w_day_of_week: u16,
        w_day: u16,
        w_hour: u16,
        w_minute: u16,
        w_second: u16,
        w_milliseconds: u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(lp_system_time: *mut SystemTime);
    }

    let mut result = SystemTime {
        w_year: 1970,
        w_month: 1,
        w_day_of_week: 0,
        w_day: 1,
        w_hour: 0,
        w_minute: 0,
        w_second: 0,
        w_milliseconds: 0,
    };
    unsafe {
        GetLocalTime(&mut result);
    }

    LocalDateTime {
        year: i32::from(result.w_year),
        month: u32::from(result.w_month),
        day: u32::from(result.w_day),
        hour: u32::from(result.w_hour),
        minute: u32::from(result.w_minute),
        second: u32::from(result.w_second),
    }
}

#[cfg(not(any(unix, windows)))]
fn local_datetime_from_unix_seconds(seconds: i64) -> LocalDateTime {
    utc_datetime_from_unix_seconds(seconds)
}

fn utc_datetime_from_unix_seconds(seconds: i64) -> LocalDateTime {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    LocalDateTime {
        year,
        month,
        day,
        hour: (seconds_of_day / 3_600) as u32,
        minute: ((seconds_of_day % 3_600) / 60) as u32,
        second: (seconds_of_day % 60) as u32,
    }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::{LogPlatform, init_at, log_error, log_path_for_platform};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn macos_log_path_uses_library_logs() {
        let path = test_log_path(
            LogPlatform::Macos,
            [("HOME", "/Users/example")],
            "/tmp",
            "20260705",
        );

        assert_eq!(
            path,
            PathBuf::from("/Users/example/Library/Logs/ascdraw/ascdraw.20260705.log")
        );
    }

    #[test]
    fn linux_log_path_uses_xdg_state_home() {
        let path = test_log_path(
            LogPlatform::Linux,
            [("XDG_STATE_HOME", "/tmp/state")],
            "/tmp",
            "20260705",
        );

        assert_eq!(
            path,
            PathBuf::from("/tmp/state/ascdraw/ascdraw.20260705.log")
        );
    }

    #[test]
    fn linux_log_path_falls_back_to_home_state_dir() {
        let path = test_log_path(
            LogPlatform::Linux,
            [("HOME", "/Users/example")],
            "/tmp",
            "20260705",
        );

        assert_eq!(
            path,
            PathBuf::from("/Users/example/.local/state/ascdraw/ascdraw.20260705.log")
        );
    }

    #[test]
    fn windows_log_path_uses_local_app_data() {
        let path = test_log_path(
            LogPlatform::Windows,
            [("LOCALAPPDATA", r"C:\Users\example\AppData\Local")],
            r"C:\Temp",
            "20260705",
        );

        assert_eq!(
            path,
            PathBuf::from(r"C:\Users\example\AppData\Local")
                .join("ascdraw")
                .join("ascdraw.20260705.log")
        );
    }

    #[test]
    fn missing_env_falls_back_to_temp_dir() {
        let path = test_log_path(LogPlatform::Macos, [], "/tmp", "20260705");

        assert_eq!(path, PathBuf::from("/tmp/ascdraw/ascdraw.20260705.log"));
    }

    #[test]
    fn init_creates_parent_directories_and_appends_log_line() {
        let path = std::env::temp_dir()
            .join(format!(
                "ascdraw-diagnostics-test-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("test time should be after unix epoch")
                    .as_nanos()
            ))
            .join("nested")
            .join("ascdraw.20260705.log");

        init_at(path.clone()).expect("diagnostics log should initialize");
        log_error("test diagnostic line");

        let contents = fs::read_to_string(&path).expect("diagnostics log should be readable");
        assert!(contents.contains("diagnostics initialized"));
        assert!(contents.contains("test diagnostic line"));
    }

    fn test_log_path<const N: usize>(
        platform: LogPlatform,
        env: [(&str, &str); N],
        temp_dir: impl AsRef<Path>,
        date: &str,
    ) -> PathBuf {
        let env: HashMap<_, _> = env
            .into_iter()
            .map(|(key, value)| (key.to_string(), OsString::from(value)))
            .collect();

        log_path_for_platform(
            platform,
            |name| env.get(name).cloned(),
            temp_dir.as_ref().to_path_buf(),
            date,
        )
    }
}
