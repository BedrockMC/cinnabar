//! Test client part shaped like SP3's ME Terminal, for the host's fuel and failure tests.
//! `dispatch` decodes the record through the SDK, as a real client part does, and by channel:
//!
//! - `terminal.panic` panics, which traps;
//! - `terminal.spin` loops until its fuel runs out;
//! - `terminal.count` counts its calls in guest memory and binds the count as the `count`
//!   collection's one row, so a fresh instance shows up as a count of 1;
//! - anything else binds its item list (records of an id, a count and a display name) into the
//!   `items` collection, one row per item, and binds nothing for any other record.

use std::sync::atomic::{AtomicI64, Ordering};

use experience_sdk::Value;
use experience_sdk::client::{ClientPart, ui};
use serde_json::{Map, json};

/// Calls of `terminal.count` since this instance started.
static CALLS: AtomicI64 = AtomicI64::new(0);

struct Terminal;

impl ClientPart for Terminal {
    fn init() {}

    fn dispatch(channel: String, record: Vec<Value>) {
        match channel.as_str() {
            "terminal.panic" => panic!("terminal fault"),
            "terminal.spin" => loop {
                std::hint::black_box(&channel);
            },
            "terminal.count" => {
                let calls = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
                let mut row = Map::new();
                row.insert("#count".into(), json!({"type": "integer", "value": calls}));
                bind("count", &[row]);
            }
            _ => items(&record),
        }
    }
}

/// Binds the records of the item list in `record` as the `items` collection.
fn items(record: &[Value]) {
    let [Value::List(items)] = record else {
        return;
    };
    let rows: Vec<Map<String, serde_json::Value>> = items
        .iter()
        .filter_map(|item| match item {
            Value::Record(fields) => match fields.as_slice() {
                [Value::Text(id), Value::Integer(count), Value::Text(name)] => {
                    Some(row(id, *count, name))
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    bind("items", &rows);
}

fn bind(collection: &str, rows: &[Map<String, serde_json::Value>]) {
    let rows = serde_json::to_vec(rows).expect("rows serialize");
    let _ = ui::set_collection(collection, &rows);
}

/// One collection row in the host's `{"#binding": {"type": …, "value": …}}` form.
fn row(id: &str, count: i64, name: &str) -> Map<String, serde_json::Value> {
    let mut row = Map::new();
    row.insert("#id".into(), json!({"type": "text", "value": id}));
    row.insert("#count".into(), json!({"type": "integer", "value": count}));
    row.insert("#name".into(), json!({"type": "text", "value": name}));
    row
}

experience_sdk::export_client_part!(Terminal);
