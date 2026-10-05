//! The controller side: one authenticated connection to a running client.

use std::{
    io::{self, BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpStream},
    path::Path,
    time::Duration,
};

use serde_json::Value;

use crate::{
    endpoint::Endpoint,
    protocol::{Command, parse_reply, request_line},
};

pub struct Controller {
    token: String,
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl Controller {
    /// Connects to the client that published `endpoint_path`.
    pub fn connect(endpoint_path: &Path) -> io::Result<Self> {
        Self::connect_to(&Endpoint::read(endpoint_path)?)
    }

    pub fn connect_to(endpoint: &Endpoint) -> io::Result<Self> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            token: endpoint.token.clone(),
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
        })
    }

    /// Sends `command` and waits up to `timeout` for its reply.
    pub fn call(&mut self, command: &Command, timeout: Duration) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let line = request_line(id, &self.token, command);
        writeln!(self.writer, "{line}")
            .and_then(|()| self.writer.flush())
            .map_err(|error| format!("send to client: {error}"))?;
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))
            .map_err(|error| error.to_string())?;
        let mut reply = String::new();
        match self.reader.read_line(&mut reply) {
            Ok(0) => return Err("client closed the control connection".into()),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(format!(
                    "no reply within {} ms; the connection is now out of step, reconnect",
                    timeout.as_millis()
                ));
            }
            Err(error) => return Err(format!("read from client: {error}")),
        }
        let (reply_id, outcome) = parse_reply(&reply)?;
        if reply_id != Value::from(id) {
            return Err(format!("reply id {reply_id} does not match request {id}"));
        }
        outcome
    }
}
