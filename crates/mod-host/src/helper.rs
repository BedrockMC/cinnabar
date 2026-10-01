//! Bounded process protocol. OS-restricted production launch deliberately fails closed.

use crate::server::BundleHost;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use server_experience::{
    crypto,
    policy::*,
    runtime::{Capabilities, Principal, Transaction},
};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const MAX_STARTUP_IPC: usize = MAX_COMPONENT_BYTES * 2 + MAX_HOST_OUTPUT;
// Decimal byte encoding needs up to four bytes per payload byte, plus bounded metadata.
const MAX_DISPATCH_IPC: usize = MAX_PAYLOAD_BYTES * 4 + MAX_HOST_OUTPUT;
const HELPER_DEADLINE: Duration = Duration::from_secs(10);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Start {
    owner: Principal,
    capabilities: Capabilities,
    epoch: u64,
    component: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Dispatch {
    pub channel: String,
    pub record: Vec<u8>,
    pub actions: BTreeSet<String>,
    pub epoch: u64,
}

pub struct Helper {
    child: Arc<Mutex<Child>>,
    requests: mpsc::SyncSender<Dispatch>,
    responses: Mutex<mpsc::Receiver<Result<Transaction>>>,
    pending_since: Option<Instant>,
    quarantined: bool,
}

impl Helper {
    /// Refuses production remote code until platform restrictions are implemented and verified.
    pub fn spawn_restricted(
        _executable: &Path,
        _bytes: &[u8],
        _owner: Principal,
        _capabilities: Capabilities,
        _epoch: u64,
    ) -> Result<Self> {
        bail!("restricted server helpers are unavailable on this build")
    }

    /// Starts a developer-only worker with empty environment and piped, bounded IPC.
    pub fn spawn_developer(
        executable: &Path,
        bytes: &[u8],
        owner: Principal,
        capabilities: Capabilities,
        epoch: u64,
    ) -> Result<Self> {
        ensure!(
            std::env::var(DEVELOPER_ENV).as_deref() == Ok("1"),
            "developer helper disabled"
        );
        ensure!(bytes.len() <= MAX_COMPONENT_BYTES, "component too large");
        let startup = Start {
            owner,
            capabilities,
            epoch,
            component: crypto::hex(bytes),
        };
        let mut child = Command::new(executable)
            .arg("server-helper")
            .env_clear()
            .env(DEVELOPER_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let (Some(mut input), Some(mut output)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            bail!("missing helper pipes");
        };
        let child = Arc::new(Mutex::new(child));
        let (requests, receiver) = mpsc::sync_channel::<Dispatch>(1);
        let (sender, responses) = mpsc::sync_channel(1);
        let helper = Self {
            child,
            requests,
            responses: Mutex::new(responses),
            pending_since: Some(Instant::now()),
            quarantined: false,
        };
        std::thread::Builder::new()
            .name("experience-ipc".into())
            .spawn(move || {
                let result = write_frame(&mut input, &startup, MAX_STARTUP_IPC)
                    .and_then(|()| read_frame(&mut output, MAX_HOST_OUTPUT));
                let failed = result.is_err();
                if sender.send(result).is_err() || failed {
                    return;
                }
                while let Ok(request) = receiver.recv() {
                    let result = write_frame(&mut input, &request, MAX_DISPATCH_IPC)
                        .and_then(|()| read_frame(&mut output, MAX_HOST_OUTPUT));
                    let failed = result.is_err();
                    if sender.send(result).is_err() || failed {
                        return;
                    }
                }
            })?;
        Ok(helper)
    }

    /// Sends one callback without ever waiting for the child from the render thread.
    pub fn dispatch(&mut self, request: Dispatch) -> Result<()> {
        ensure!(
            !self.quarantined && self.pending_since.is_none(),
            "helper busy or quarantined"
        );
        ensure!(
            request.record.len() <= MAX_PAYLOAD_BYTES
                && serde_json::to_vec(&request)?.len() <= MAX_DISPATCH_IPC,
            "helper event too large"
        );
        self.requests.try_send(request)?;
        self.pending_since = Some(Instant::now());
        Ok(())
    }

    /// Polls completed output and kills a stalled compiler or guest after its deadline.
    pub fn poll(&mut self) -> Option<Result<Transaction>> {
        if self.quarantined {
            return None;
        }
        if self
            .pending_since
            .is_some_and(|since| since.elapsed() >= HELPER_DEADLINE)
        {
            self.kill();
            return Some(Err(anyhow::anyhow!("helper deadline exceeded")));
        }
        let response = self.responses.lock().ok()?.try_recv();
        match response {
            Ok(result) => {
                self.pending_since = None;
                if result.is_err() {
                    self.kill();
                }
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.kill();
                Some(Err(anyhow::anyhow!("helper exited")))
            }
        }
    }

    /// Revokes this process immediately; no automatic restart is allowed.
    fn kill(&mut self) {
        self.quarantined = true;
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

impl Drop for Helper {
    /// Ends guest execution before deferring process reaping off the main thread.
    fn drop(&mut self) {
        self.kill();
        let child = Arc::clone(&self.child);
        let _ = std::thread::Builder::new()
            .name("experience-reap".into())
            .spawn(move || {
                if let Ok(mut child) = child.lock() {
                    let _ = child.wait();
                }
            });
    }
}

/// Runs only as the private helper entry point; there are no inherited game handles.
pub fn serve_developer() -> Result<()> {
    ensure!(
        std::env::var(DEVELOPER_ENV).as_deref() == Ok("1"),
        "developer helper disabled"
    );
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let startup: Start = read_frame(&mut input, MAX_STARTUP_IPC)?;
    ensure!(
        startup.component.len() <= MAX_COMPONENT_BYTES * 2,
        "component too large"
    );
    let mut host = BundleHost::instantiate(
        &crypto::unhex(&startup.component)?,
        startup.owner,
        startup.capabilities,
        startup.epoch,
    )?;
    write_frame(&mut output, &host.take_transaction(), MAX_HOST_OUTPUT)?;
    loop {
        let request: Dispatch = read_frame(&mut input, MAX_DISPATCH_IPC)?;
        ensure!(
            request.record.len() <= MAX_PAYLOAD_BYTES,
            "helper payload too large"
        );
        let result = host.dispatch(
            &request.channel,
            &request.record,
            request.actions,
            request.epoch,
        )?;
        write_frame(&mut output, &result, MAX_HOST_OUTPUT)?;
    }
}

/// Checks an IPC length before allocating or deserializing its payload.
fn read_frame<T: serde::de::DeserializeOwned>(reader: &mut impl Read, limit: usize) -> Result<T> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_le_bytes(header) as usize;
    ensure!(len <= limit, "IPC frame too large");
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Sends one length-delimited transaction without ambient handles or paths.
fn write_frame(writer: &mut impl Write, value: &impl Serialize, limit: usize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= limit, "IPC frame too large");
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_typed_payload_round_trips_through_dispatch_ipc() {
        let empty = serde_json::to_vec(&vec![server_experience::wire::Scalar::Text(String::new())])
            .unwrap()
            .len();
        let record = serde_json::to_vec(&vec![server_experience::wire::Scalar::Text(
            "x".repeat(MAX_PAYLOAD_BYTES - empty),
        )])
        .unwrap();
        assert_eq!(record.len(), MAX_PAYLOAD_BYTES);
        let request = Dispatch {
            channel: "fixture.events".into(),
            record,
            actions: BTreeSet::new(),
            epoch: u64::MAX,
        };
        assert!(serde_json::to_vec(&request).unwrap().len() > MAX_HOST_OUTPUT);
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &request, MAX_DISPATCH_IPC).unwrap();
        let decoded: Dispatch = read_frame(&mut bytes.as_slice(), MAX_DISPATCH_IPC).unwrap();
        assert_eq!(decoded.record, request.record);
    }
}
