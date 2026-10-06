//! One discovery record per running process.
//!
//! The directory is `$XDG_RUNTIME_DIR/genos/mcp` when that variable is set.
//! Otherwise it is `/tmp/genos-mcp-<uid>`. Each file stores the process id,
//! the Linux start time, and the MCP endpoint URL. A record whose process is
//! gone is not a live endpoint.

use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{self, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub pid: u32,
    pub url: String,
}

pub fn discovery_dir() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime).join("genos").join("mcp");
        }
    }
    std::env::temp_dir().join(format!("genos-mcp-{}", current_uid()))
}

/// Live endpoints only. A record for a dead process is removed.
pub fn live_endpoints() -> Vec<Endpoint> {
    let directory = discovery_dir();
    let Ok(entries) = fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = json::parse(&text) else {
            continue;
        };
        let Some(pid) = value.get("pid").and_then(Value::as_u64) else {
            continue;
        };
        let pid = pid as u32;
        let starttime = value.get("starttime").and_then(Value::as_u64);
        if !process_alive(pid, starttime) {
            let _ = fs::remove_file(&path);
            continue;
        }
        let Some(url) = value.get("url").and_then(Value::as_str) else {
            continue;
        };
        if !url.starts_with("http://127.0.0.1:") && !url.starts_with("http://localhost:") {
            continue;
        }
        found.push(Endpoint {
            pid,
            url: url.to_string(),
        });
    }
    found.sort_by_key(|endpoint| endpoint.pid);
    found
}

/// Read one record and refuse it when that process is gone.
pub fn endpoint_at(path: &Path) -> Result<Endpoint, String> {
    let text = fs::read_to_string(path).map_err(|_| "discovery record is missing".to_string())?;
    let value = json::parse(&text).map_err(|_| "discovery record is unreadable".to_string())?;
    let pid = value
        .get("pid")
        .and_then(Value::as_u64)
        .ok_or("discovery record has no process")? as u32;
    let starttime = value.get("starttime").and_then(Value::as_u64);
    if !process_alive(pid, starttime) {
        return Err("process is gone".into());
    }
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or("discovery record has no endpoint")?;
    Ok(Endpoint {
        pid,
        url: url.to_string(),
    })
}

pub fn publish(url: &str) -> Result<(), String> {
    let directory = discovery_dir();
    fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    let pid = std::process::id();
    let starttime = process_starttime(pid).unwrap_or(0);
    let body = json::encode(&json::object([
        ("pid", json::int(pid as i64)),
        ("starttime", json::int(starttime as i64)),
        ("url", json::string(url)),
    ]));
    let path = record_path(pid);
    let temporary = directory.join(format!(".{pid}.json.tmp"));
    fs::write(&temporary, body).map_err(|err| err.to_string())?;
    fs::rename(&temporary, &path).map_err(|err| err.to_string())?;
    Ok(())
}

pub fn remove_record_if_url(url: &str) {
    let path = record_path(std::process::id());
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = json::parse(&text) else {
        return;
    };
    if value.get("url").and_then(Value::as_str) == Some(url) {
        let _ = fs::remove_file(path);
    }
}

fn record_path(pid: u32) -> PathBuf {
    discovery_dir().join(format!("{pid}.json"))
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    unsafe {
        geteuid()
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(unix)]
extern "C" {
    fn geteuid() -> u32;
}

fn process_alive(pid: u32, starttime: Option<u64>) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let Some(actual) = linux_starttime(pid) else {
            return false;
        };
        return match starttime {
            Some(expected) if expected != 0 => actual == expected,
            _ => true,
        };
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let _ = starttime;
        let rc = unsafe { kill(pid as i32, 0) };
        return rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(1);
    }
    #[cfg(windows)]
    {
        let _ = starttime;
        return windows_process_alive(pid);
    }
}

#[cfg(target_os = "linux")]
fn process_starttime(pid: u32) -> Option<u64> {
    linux_starttime(pid)
}

#[cfg(not(target_os = "linux"))]
fn process_starttime(pid: u32) -> Option<u64> {
    let _ = pid;
    None
}

#[cfg(target_os = "linux")]
fn linux_starttime(pid: u32) -> Option<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = text.rfind(')')?;
    let fields: Vec<&str> = text[rest + 1..].split_whitespace().collect();
    // Field 22 of `stat` is starttime. The comm field ends at the last `)`.
    fields.get(19)?.parse().ok()
}

#[cfg(all(unix, not(target_os = "linux")))]
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(windows)]
fn windows_process_alive(pid: u32) -> bool {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut core::ffi::c_void;
        fn CloseHandle(handle: *mut core::ffi::c_void) -> i32;
        fn GetLastError() -> u32;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == 5;
        }
        CloseHandle(handle);
        true
    }
}
