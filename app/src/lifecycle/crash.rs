//! Opt-in crash reporting: a panic hook records a scrubbed report; the core uploads it on a later launch.

use std::{
    backtrace::Backtrace,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{install_layout::InstallLayout, native_dialog::Prompter};

const DSN_ENV: &str = "CINNABAR_SENTRY_DSN";
const OPT_IN_ENV: &str = "CINNABAR_CRASH_REPORTS";
const LOG_TAIL_BYTES: u64 = 16 * 1024;
const MAX_PENDING: usize = 8;

/// Matches the core's crash-report schema.
#[derive(Debug, Serialize)]
struct Report<'a> {
    source: &'a str,
    message: String,
    backtrace: String,
    log_tail: String,
    release: &'a str,
    os: &'a str,
    arch: &'a str,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Consent {
    enabled: Option<bool>,
}

fn consent_path(layout: &InstallLayout) -> PathBuf {
    layout.user_config_root.join("crash-reporting.json")
}

fn read_consent(layout: &InstallLayout) -> Option<bool> {
    match std::env::var(OPT_IN_ENV).as_deref() {
        Ok("1") => return Some(true),
        Ok("0") => return Some(false),
        _ => {}
    }
    let bytes = fs::read(consent_path(layout)).ok()?;
    serde_json::from_slice::<Consent>(&bytes).ok()?.enabled
}

fn write_consent(layout: &InstallLayout, enabled: bool) {
    let path = consent_path(layout);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec(&Consent {
        enabled: Some(enabled),
    }) {
        let _ = fs::write(path, bytes);
    }
}

/// Records a report for every panic; nothing leaves the machine without opt-in.
pub(crate) fn install_panic_hook(layout: &InstallLayout) {
    let crash_dir = layout.crash_dir();
    let core_log = layout.log_dir().join("core.log");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_report(&crash_dir, &core_log, &info.to_string());
        previous(info);
    }));
}

fn write_report(crash_dir: &Path, core_log: &Path, message: &str) {
    let report = Report {
        source: "client",
        message: message.to_owned(),
        backtrace: Backtrace::force_capture().to_string(),
        log_tail: tail(core_log, LOG_TAIL_BYTES),
        release: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    };
    let Ok(bytes) = serde_json::to_vec(&report) else {
        return;
    };
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    if fs::create_dir_all(crash_dir).is_ok() {
        let _ = fs::write(crash_dir.join(format!("crash-{millis}.json")), bytes);
    }
}

fn tail(path: &Path, limit: u64) -> String {
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map_or(0, |meta| meta.len());
    let _ = file.seek(SeekFrom::Start(length.saturating_sub(limit)));
    let mut bytes = Vec::new();
    let _ = file.take(limit).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn pending(crash_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(crash_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

fn dsn(layout: &InstallLayout) -> Option<String> {
    std::env::var(DSN_ENV)
        .ok()
        .or_else(|| fs::read_to_string(layout.resource_root.join("sentry-dsn")).ok())
        .map(|dsn| dsn.trim().to_owned())
        .filter(|dsn| !dsn.is_empty())
}

/// Uploads queued reports through the core if the user opted in; asks once when a report exists and no choice is recorded.
pub(crate) fn upload_pending(layout: &InstallLayout, prompter: &dyn Prompter) {
    let mut files = pending(&layout.crash_dir());
    if files.is_empty() {
        return;
    }
    // Oldest reports beyond the bound are dropped so a crash loop cannot grow the queue.
    while files.len() > MAX_PENDING {
        let _ = fs::remove_file(files.remove(0));
    }
    let Some(dsn) = dsn(layout) else {
        return;
    };
    let enabled = read_consent(layout).unwrap_or_else(|| {
        let answer = prompter.confirm(
            "Send crash report?",
            "Cinnabar closed unexpectedly last time. Send an anonymous crash report (error details, version, operating system) to help fix it?",
        );
        write_consent(layout, answer);
        answer
    });
    if !enabled {
        for file in files {
            let _ = fs::remove_file(file);
        }
        return;
    }
    let core = layout.core_executable.clone();
    std::thread::spawn(move || {
        for file in files {
            let uploaded = Command::new(&core)
                .args(["upload-crash", "-file"])
                .arg(&file)
                .env(DSN_ENV, &dsn)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if uploaded {
                let _ = fs::remove_file(&file);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cinnabar-crash-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn report_matches_the_core_schema_and_carries_the_log_tail() {
        let dir = scratch("report");
        let log = dir.join("core.log");
        fs::write(&log, "old\nrecent line").unwrap();
        write_report(&dir.join("crashes"), &log, "boom");
        let file = pending(&dir.join("crashes")).remove(0);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
        for key in [
            "source",
            "message",
            "backtrace",
            "log_tail",
            "release",
            "os",
            "arch",
        ] {
            assert!(value.get(key).is_some(), "missing {key}");
        }
        assert_eq!(value["source"], "client");
        assert!(value["log_tail"].as_str().unwrap().contains("recent line"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tail_is_bounded_to_the_final_bytes() {
        let dir = scratch("tail");
        let log = dir.join("l");
        fs::write(&log, "0123456789").unwrap();
        assert_eq!(tail(&log, 4), "6789");
        assert_eq!(tail(&dir.join("missing"), 4), "");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pending_lists_only_json_reports_in_order() {
        let dir = scratch("pending");
        for name in ["crash-2.json", "crash-1.json", "note.txt"] {
            fs::write(dir.join(name), "{}").unwrap();
        }
        let names: Vec<_> = pending(&dir)
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["crash-1.json", "crash-2.json"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
