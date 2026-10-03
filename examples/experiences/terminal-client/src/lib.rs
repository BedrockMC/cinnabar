//! Test client part shaped like SP3's ME Terminal, for the host's fuel tests: `dispatch` decodes
//! the record through the SDK, as a real client part does, and binds its item list (records of an
//! id, a count and a display name) into the `items` collection, one row per item. Anything else
//! binds nothing.

use experience_sdk::Value;
use experience_sdk::client::{ClientPart, ui};
use serde_json::{Map, json};

struct Terminal;

impl ClientPart for Terminal {
    fn init() {}

    fn dispatch(_channel: String, record: Vec<Value>) {
        let [Value::List(items)] = record.as_slice() else {
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
        let rows = serde_json::to_vec(&rows).expect("rows serialize");
        let _ = ui::set_collection("items", &rows);
    }
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
