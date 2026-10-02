//! The stdio session of one Experience. The adapter's first frame loads it; each callback frame
//! then gets a result with the same `seq`, until `shutdown` or the end of input. Only frames go
//! to the output; logs go to stderr.

use std::fmt::Display;
use std::io::{self, ErrorKind, Read, Write};
use std::path::Path;

use anyhow::Result;
use wasmtime::Engine;

use crate::callback;
use crate::load::{self, EpochTicker, Loaded};
use crate::protocol::{
    Call, FailKind, Outcome, PROTOCOL_VERSION, Request, Response, read_frame, write_frame,
};

/// The session ended with `shutdown`, or with the end of input between frames.
pub const EXIT_OK: i32 = 0;
/// The artifact did not load, and the adapter was answered `load_failed`.
pub const EXIT_LOAD_FAILED: i32 = 1;
/// A frame did not decode, or was not the request expected at its point in the session.
pub const EXIT_PROTOCOL: i32 = 2;

/// Why a result that is too large for a frame failed instead.
const OVERSIZED_RESULT: &str = "result exceeds the frame limit";

/// Serves one session, reading requests from `input` and writing responses to `output`, and
/// returns the process exit code. With `report_fuel`, each callback logs the fuel it consumed.
/// An error is a response that could not be written.
pub fn serve(mut input: impl Read, mut output: impl Write, report_fuel: bool) -> Result<i32> {
    let dir = match read_frame(&mut input) {
        Ok(Some(Request::Load { dir })) => dir,
        Ok(Some(Request::Callback { .. } | Request::Shutdown {})) => {
            return Ok(protocol_error("the first frame is not load"));
        }
        Ok(None) => return Ok(EXIT_OK),
        Err(error) => return Ok(protocol_error(error)),
    };
    // The ticker drives every deadline, so it lives as long as the session.
    let (engine, _ticker, loaded) = match start(Path::new(&dir)) {
        Ok(started) => started,
        Err(error) => {
            let reason = format!("{error:#}");
            eprintln!("serve: {reason}");
            write_frame(&mut output, &Response::LoadFailed { reason })?;
            return Ok(EXIT_LOAD_FAILED);
        }
    };
    let response = Response::Loaded {
        protocol: PROTOCOL_VERSION,
        id: loaded.manifest.id.clone(),
        version: loaded.manifest.version.clone(),
        blocks: loaded.blocks.clone(),
    };
    write_frame(&mut output, &response)?;
    loop {
        let request = match read_frame(&mut input) {
            Ok(Some(request)) => request,
            Ok(None) => return Ok(EXIT_OK),
            Err(error) => return Ok(protocol_error(error)),
        };
        let (seq, call) = match &request {
            Request::Callback { seq, call, .. } => (*seq, call),
            Request::Shutdown {} => return Ok(EXIT_OK),
            Request::Load { .. } => return Ok(protocol_error("load after the artifact loaded")),
        };
        let (outcome, fuel) = callback::run_metered(&engine, &loaded, &request);
        if report_fuel {
            eprintln!("fuel {} {} {fuel}", loaded.manifest.id, call_kind(call));
        }
        answer(&mut output, seq, outcome)?;
    }
}

/// The engine with its ticker, and the artifact in `dir` loaded on it.
fn start(dir: &Path) -> Result<(Engine, EpochTicker, Loaded)> {
    let (engine, ticker) = load::engine()?;
    let loaded = load::load(&engine, dir)?;
    Ok((engine, ticker, loaded))
}

/// Logs why the session ends without an answer, and returns [`EXIT_PROTOCOL`].
fn protocol_error(error: impl Display) -> i32 {
    eprintln!("serve: protocol error: {error}");
    EXIT_PROTOCOL
}

/// Writes the result of callback `seq`. A result too large for a frame fails as a limit
/// instead, so the callback is still answered.
fn answer(output: &mut impl Write, seq: u64, outcome: Outcome) -> io::Result<()> {
    match write_frame(output, &Response::Result { seq, outcome }) {
        Err(error) if error.kind() == ErrorKind::InvalidInput => {
            eprintln!("serve: result {seq}: {error}");
            let outcome = Outcome::Failed {
                kind: FailKind::Limit,
                reason: OVERSIZED_RESULT.to_owned(),
            };
            write_frame(output, &Response::Result { seq, outcome })
        }
        written => written,
    }
}

/// The `type` of `call` in the protocol.
fn call_kind(call: &Call) -> &'static str {
    match call {
        Call::Place { .. } => "place",
        Call::Break { .. } => "break",
        Call::Interact { .. } => "interact",
        Call::Neighbor { .. } => "neighbor",
    }
}

#[cfg(test)]
mod tests {
    use super::{OVERSIZED_RESULT, answer};
    use crate::limits::MAX_FRAME_BYTES;
    use crate::protocol::{FailKind, Op, Outcome, Response, read_frame};

    /// A result too large for a frame is answered as a `limit` failure instead, and nothing of
    /// it is written.
    #[test]
    fn oversized_result_fails_with_limit() {
        let tell = Op::Tell {
            player: String::new(),
            text: "x".repeat(MAX_FRAME_BYTES),
        };
        let mut output = Vec::new();
        answer(&mut output, 7, Outcome::Committed { ops: vec![tell] }).unwrap();
        let mut frames = output.as_slice();
        let failed = Response::Result {
            seq: 7,
            outcome: Outcome::Failed {
                kind: FailKind::Limit,
                reason: OVERSIZED_RESULT.to_owned(),
            },
        };
        assert_eq!(read_frame(&mut frames).unwrap(), Some(failed));
        assert_eq!(read_frame::<Response>(&mut frames).unwrap(), None);
    }
}
