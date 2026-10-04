//! A stdio MCP server over the JSON-UI editor core: load packs, list screens,
//! resolve with provenance, validate, lay out and render to PNG, headlessly.
//! Speaks newline-delimited JSON-RPC 2.0.

#[cfg(test)]
mod tests;
mod tools;

use std::io::{BufRead, Write};

use serde_json::{Value, json};

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

fn main() {
    let mut server = tools::Server::new(font_argument());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle(&mut server, &message),
            Err(error) => Some(failure(Value::Null, -32700, &error.to_string())),
        };
        if let Some(reply) = reply
            && (writeln!(stdout, "{reply}").is_err() || stdout.flush().is_err())
        {
            break;
        }
    }
}

/// `--font <carrier>`: the compiled Cinnangles Sans carrier text measures and draws with.
fn font_argument() -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--font" {
            return args.next();
        }
        if let Some(path) = arg.strip_prefix("--font=") {
            return Some(path.to_owned());
        }
    }
    None
}

/// The reply to one message; notifications get none.
pub fn handle(server: &mut tools::Server, message: &Value) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|version| PROTOCOL_VERSIONS.contains(version))
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": requested,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "jsonui-mcp", "version": env!("CARGO_PKG_VERSION") },
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools::definitions() }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            server.call(name, &arguments)
        }
        _ => return Some(failure(id, -32601, &format!("unknown method `{method}`"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
