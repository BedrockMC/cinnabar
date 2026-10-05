use std::{path::Path, time::Duration};

use developer_control::{
    endpoint::Endpoint,
    protocol::{Command, Condition},
    server::ControlServer,
};
use mcp_stdio::{ToolServer, handle};
use serde_json::{Value, json};

use crate::{commands, tools::Server};

fn scratch(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("cinnabar-mcp-{label}-{}", std::process::id()))
}

fn result_json(reply: &Value) -> Value {
    serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn tools_list_names_every_tool() {
    let mut server = Server::new(scratch("list"));
    let reply = handle(
        &mut server,
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    )
    .unwrap();
    let names: Vec<&str> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "launch_client",
            "connect",
            "input",
            "chat",
            "camera_path",
            "state",
            "wait_for",
            "screenshot",
            "record_start",
            "record_stop",
            "quit"
        ]
    );
    let info = handle(&mut server, &json!({ "id": 2, "method": "initialize" })).unwrap();
    assert_eq!(info["result"]["serverInfo"]["name"], "cinnabar-mcp");
}

#[test]
fn tools_without_a_client_fail_with_guidance() {
    let mut server = Server::new(scratch("idle"));
    let reply = server.call("state", &json!({}));
    assert_eq!(reply["isError"], true);
    assert!(
        result_json(&reply)["error"]
            .as_str()
            .unwrap()
            .contains("launch_client")
    );
}

#[test]
fn remote_addresses_need_an_explicit_opt_in() {
    assert!(commands::is_loopback("127.0.0.1:19132"));
    assert!(commands::is_loopback("localhost:19132"));
    assert!(commands::is_loopback("[::1]:19132"));
    assert!(!commands::is_loopback("zeqa.net:19132"));
    assert!(!commands::is_loopback("10.0.0.2:19132"));
    let mut server = Server::new(scratch("remote"));
    let reply = server.call("connect", &json!({ "address": "play.example.net:19132" }));
    assert_eq!(reply["isError"], true);
    assert!(
        result_json(&reply)["error"]
            .as_str()
            .unwrap()
            .contains("allow_remote")
    );
}

#[test]
fn tool_arguments_become_commands_with_resolved_paths() {
    let repo = Path::new("/repo");
    let control = repo.join(".local/developer-control");
    let (command, timeout) = commands::command(
        "wait_for",
        &json!({ "condition": "in_world", "timeout_ms": 1000 }),
        repo,
        &control,
    )
    .unwrap();
    assert_eq!(
        command,
        Command::WaitFor {
            condition: Condition::InWorld,
            timeout_ms: Some(1000)
        }
    );
    assert!(timeout > Duration::from_millis(1000));
    let (command, _) = commands::command(
        "screenshot",
        &json!({ "path": "shots/a.png" }),
        repo,
        &control,
    )
    .unwrap();
    assert_eq!(
        command,
        Command::Screenshot {
            path: repo.join("shots/a.png")
        }
    );
    let (command, _) = commands::command("screenshot", &json!({}), repo, &control).unwrap();
    let Command::Screenshot { path } = command else {
        panic!("not a screenshot")
    };
    assert!(path.starts_with(control.join("screenshots")));
    let (command, _) =
        commands::command("record_start", &json!({ "fps": 30 }), repo, &control).unwrap();
    let Command::RecordStart(settings) = command else {
        panic!("not record_start")
    };
    assert_eq!(settings.fps, 30);
    assert!(settings.path.starts_with(control.join("recordings")));
    assert!(commands::command("record_start", &json!({ "fps": 0 }), repo, &control).is_err());
    let (command, _) =
        commands::command("camera_path", &json!({ "release": true }), repo, &control).unwrap();
    assert_eq!(command, Command::CameraRelease);
    assert!(commands::command("camera_path", &json!({ "keyframes": [] }), repo, &control).is_err());
    assert!(commands::command("input", &json!({ "jumpp": true }), repo, &control).is_err());
}

#[test]
fn attached_servers_relay_commands_and_replies() {
    let endpoint_path = scratch("attach").join("endpoint.json");
    let control = ControlServer::start(&endpoint_path).unwrap();
    let game_loop = std::thread::spawn(move || {
        for _ in 0..2_000 {
            let pending = control.drain().next();
            if let Some(pending) = pending {
                let outcome = match pending.command {
                    Command::Chat { text } => Ok(json!({ "sent": text })),
                    other => Err(format!("unexpected {other:?}")),
                };
                pending.reply.send(outcome);
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no command arrived");
    });
    let mut server = Server::new(scratch("attach-repo"));
    server
        .attach(&Endpoint::read(&endpoint_path).unwrap())
        .unwrap();
    let reply = server.call("chat", &json!({ "text": "/showcase souls" }));
    assert_eq!(reply["isError"], false, "{reply}");
    assert_eq!(result_json(&reply)["sent"], "/showcase souls");
    game_loop.join().unwrap();
}
