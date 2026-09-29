//! Remote form images (`http`/`https` button images) downloaded off the frame
//! loop into a bounded in-memory cache, as the vanilla client fetches them.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, Weak, mpsc},
    time::Duration,
};

/// Most URLs remembered; the oldest settled entry goes first.
const MAX_ENTRIES: usize = 64;
/// Largest response body accepted for one image.
const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub(super) enum RemoteState {
    Loading,
    Ready(Arc<[u8]>),
    Failed,
}

/// A shared handle to the download cache and its single worker thread.
#[derive(Clone, Default)]
pub(super) struct RemoteImages(Arc<Remote>);

#[derive(Default)]
struct Remote {
    entries: Mutex<Entries>,
    worker: Mutex<Option<mpsc::Sender<String>>>,
}

#[derive(Default)]
struct Entries {
    states: HashMap<String, RemoteState>,
    order: VecDeque<String>,
}

pub(super) fn is_remote(path: &str) -> bool {
    path.starts_with("https://") || path.starts_with("http://")
}

impl RemoteImages {
    /// The download state of `url`, starting its download on first sight.
    pub(super) fn state(&self, url: &str) -> RemoteState {
        let mut entries = self.0.entries.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(state) = entries.states.get(url) {
            return state.clone();
        }
        let valid =
            url::Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"));
        let state = if valid && self.request(url) {
            RemoteState::Loading
        } else {
            RemoteState::Failed
        };
        entries.states.insert(url.to_owned(), state.clone());
        entries.order.push_back(url.to_owned());
        while entries.order.len() > MAX_ENTRIES {
            let Some(position) = entries
                .order
                .iter()
                .position(|key| entries.states.get(key) != Some(&RemoteState::Loading))
            else {
                break;
            };
            if let Some(key) = entries.order.remove(position) {
                entries.states.remove(&key);
            }
        }
        state
    }

    fn request(&self, url: &str) -> bool {
        let mut worker = self.0.worker.lock().unwrap_or_else(|p| p.into_inner());
        if worker.is_none() {
            *worker = spawn(Arc::downgrade(&self.0));
        }
        worker
            .as_ref()
            .is_some_and(|sender| sender.send(url.to_owned()).is_ok())
    }
}

impl Remote {
    fn finish(&self, url: String, bytes: Option<Vec<u8>>) {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(state) = entries.states.get_mut(&url) {
            *state = bytes.map_or(RemoteState::Failed, |bytes| {
                RemoteState::Ready(bytes.into())
            });
        }
    }
}

/// One worker fetching queued URLs in order; it ends with the cache.
fn spawn(remote: Weak<Remote>) -> Option<mpsc::Sender<String>> {
    let (sender, receiver) = mpsc::channel::<String>();
    std::thread::Builder::new()
        .name("form-images".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let Ok(client) = reqwest::Client::builder().timeout(FETCH_TIMEOUT).build() else {
                return;
            };
            for url in receiver {
                let bytes = runtime.block_on(fetch(&client, &url));
                let Some(remote) = remote.upgrade() else {
                    break;
                };
                remote.finish(url, bytes);
            }
        })
        .ok()?;
    Some(sender)
}

async fn fetch(client: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let mut response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_IMAGE_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Serves `body` for every request on a local port; returns its base URL.
    pub(crate) fn serve(body: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut request = [0; 1024];
                let _ = stream.read(&mut request);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{address}")
    }

    pub(crate) fn settle(images: &RemoteImages, url: &str) -> RemoteState {
        for _ in 0..250 {
            let state = images.state(url);
            if state != RemoteState::Loading {
                return state;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        images.state(url)
    }

    // A URL downloads once in the background; other schemes never fetch.
    #[test]
    fn remote_images_download_in_the_background() {
        let base = serve(b"image bytes".to_vec());
        let images = RemoteImages::default();
        let url = format!("{base}/button.png");
        assert_eq!(
            settle(&images, &url),
            RemoteState::Ready(b"image bytes".as_slice().into())
        );
        assert_eq!(images.state("file:///etc/passwd"), RemoteState::Failed);
    }
}
