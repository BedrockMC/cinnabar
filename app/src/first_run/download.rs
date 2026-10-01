//! Resumable download of the pinned sample-pack archive with byte progress. The fetch script
//! reuses the archive once it hash-verifies, so it only extracts and publishes.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use reqwest::{StatusCode, header::RANGE};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::runner::Cancelled;

const REPORT_INTERVAL: Duration = Duration::from_millis(100);
/// Where the fetch script looks for a verified archive, relative to the workspace.
const DOWNLOADS: &str = ".local/assets/downloads";

#[derive(Deserialize)]
struct Source {
    url: String,
    sha256: String,
    archive: String,
}

/// Leaves the verified archive where `fetch-vanilla-assets` looks for it; `progress` receives
/// (bytes received, bytes expected).
pub(super) fn fetch_archive(
    workspace: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<()> {
    let manifest = workspace.join(super::plan::VANILLA_MANIFEST);
    let bytes = fs::read(&manifest).with_context(|| format!("read {}", manifest.display()))?;
    let source: Source =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", manifest.display()))?;
    if !source.url.starts_with("https://") {
        bail!("sample pack URL is not HTTPS: {}", source.url);
    }
    let expected = source.sha256.to_ascii_lowercase();
    let target = archive_path(workspace, &source.archive);
    if target.is_file() && sha256_file(&target)? == expected {
        let len = fs::metadata(&target)?.len();
        progress(len, Some(len));
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let partial = target.with_file_name(format!("{}.partial", source.archive));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime
        .block_on(download_to(&source.url, &partial, cancel, &mut progress))
        .map_err(|error| {
            if error.is::<Cancelled>() {
                error
            } else {
                error.context("Could not download the Minecraft resources. Check your internet connection, then retry")
            }
        })?;
    let actual = sha256_file(&partial)?;
    if actual != expected {
        let _ = fs::remove_file(&partial);
        bail!(
            "the downloaded pack failed verification (SHA-256 {actual}); retry to download it again"
        );
    }
    fs::rename(&partial, &target)
        .with_context(|| format!("move {} to {}", partial.display(), target.display()))
}

/// Deletes every download except the current pin's verified archive.
pub(super) fn prune(workspace: &Path) {
    let keep = fs::read(workspace.join(super::plan::VANILLA_MANIFEST))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Source>(&bytes).ok())
        .map(|source| source.archive);
    let Ok(entries) = fs::read_dir(workspace.join(DOWNLOADS)) else {
        return;
    };
    for entry in entries.flatten() {
        if keep.as_deref() != entry.file_name().to_str() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn archive_path(workspace: &Path, archive: &str) -> PathBuf {
    workspace.join(DOWNLOADS).join(archive)
}

/// Appends to `partial` when the server honours a range request, else starts it over.
async fn download_to(
    url: &str,
    partial: &Path,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(u64, Option<u64>),
) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()?;
    let resume_from = fs::metadata(partial).map_or(0, |meta| meta.len());
    let mut request = client.get(url);
    if resume_from > 0 {
        request = request.header(RANGE, format!("bytes={resume_from}-"));
    }
    let mut response = request
        .send()
        .await
        .context("connect to the download server")?;
    let (mut file, mut received) = match response.status() {
        StatusCode::PARTIAL_CONTENT if resume_from > 0 => {
            (OpenOptions::new().append(true).open(partial)?, resume_from)
        }
        // The partial file already holds every byte; verification decides whether it is good.
        StatusCode::RANGE_NOT_SATISFIABLE if resume_from > 0 => return Ok(()),
        status if status.is_success() => (File::create(partial)?, 0),
        status => bail!("the download server answered {status}"),
    };
    let total = response
        .content_length()
        .map(|remaining| remaining + received);
    progress(received, total);
    let mut reported = Instant::now();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("the download was interrupted")?
    {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        file.write_all(&chunk)
            .with_context(|| format!("write {}", partial.display()))?;
        received += chunk.len() as u64;
        if reported.elapsed() >= REPORT_INTERVAL {
            progress(received, total);
            reported = Instant::now();
        }
    }
    file.sync_all()?;
    progress(received, total);
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        thread,
    };

    use super::*;
    use crate::first_run::test_support::Dir;

    const BODY: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

    /// Serves `BODY` once, honouring `Range: bytes=N-` only when `ranges` is set.
    fn serve_once(ranges: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut start = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                    start = value.trim().trim_end_matches('-').parse().unwrap();
                }
                if line.trim().is_empty() {
                    break;
                }
            }
            let (status, body) = if ranges && start > 0 {
                ("206 Partial Content", &BODY[start..])
            } else {
                ("200 OK", BODY)
            };
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        format!("http://{address}/pack.zip")
    }

    fn download(url: &str, partial: &Path) -> Vec<(u64, Option<u64>)> {
        let mut seen = Vec::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(download_to(
                url,
                partial,
                &AtomicBool::new(false),
                &mut |r, t| {
                    seen.push((r, t));
                },
            ))
            .unwrap();
        seen
    }

    #[test]
    fn a_partial_download_resumes_from_its_length() {
        let dir = Dir::new("download-resume");
        let partial = dir.path().join("pack.zip.partial");
        fs::write(&partial, &BODY[..10]).unwrap();
        let seen = download(&serve_once(true), &partial);
        assert_eq!(fs::read(&partial).unwrap(), BODY);
        assert_eq!(seen.first(), Some(&(10, Some(BODY.len() as u64))));
    }

    #[test]
    fn a_server_ignoring_ranges_restarts_the_file() {
        let dir = Dir::new("download-restart");
        let partial = dir.path().join("pack.zip.partial");
        fs::write(&partial, b"stale-bytes").unwrap();
        download(&serve_once(false), &partial);
        assert_eq!(fs::read(&partial).unwrap(), BODY);
    }

    #[test]
    fn pruning_keeps_only_the_current_archive() {
        let dir = Dir::new("download-prune");
        let downloads = dir.path().join(DOWNLOADS);
        fs::create_dir_all(&downloads).unwrap();
        fs::create_dir_all(dir.path().join("assets")).unwrap();
        fs::write(
            dir.path().join(super::super::plan::VANILLA_MANIFEST),
            r#"{"url":"https://x/new.zip","sha256":"00","archive":"new.zip"}"#,
        )
        .unwrap();
        for name in ["old.zip", "new.zip", "new.zip.partial"] {
            fs::write(downloads.join(name), b"x").unwrap();
        }
        prune(dir.path());
        let left: Vec<_> = fs::read_dir(&downloads)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["new.zip"]);
    }

    #[test]
    fn a_verified_archive_is_reused_without_a_request() {
        let dir = Dir::new("download-reuse");
        let archive = archive_path(dir.path(), "pack.zip");
        fs::create_dir_all(archive.parent().unwrap()).unwrap();
        fs::write(&archive, BODY).unwrap();
        fs::create_dir_all(dir.path().join("assets")).unwrap();
        let sha = format!("{:x}", Sha256::digest(BODY));
        fs::write(
            dir.path().join(super::super::plan::VANILLA_MANIFEST),
            format!(
                r#"{{"url":"https://127.0.0.1:9/none","sha256":"{sha}","archive":"pack.zip"}}"#
            ),
        )
        .unwrap();
        let mut seen = None;
        fetch_archive(dir.path(), &AtomicBool::new(false), |r, t| {
            seen = Some((r, t))
        })
        .unwrap();
        assert_eq!(seen, Some((BODY.len() as u64, Some(BODY.len() as u64))));
    }
}
