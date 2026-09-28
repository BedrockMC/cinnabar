//! Block-state properties parsed from the registry's canonical state JSON.

use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
enum StateValue {
    Int(i64),
    Text(Box<str>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct BlockState(BTreeMap<Box<str>, StateValue>);

impl BlockState {
    /// Accepts typed (`{"type": .., "value": ..}`) and plain values; anything else is ignored.
    pub(super) fn parse(canonical: &str) -> Self {
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(canonical) else {
            return Self::default();
        };
        let values = map
            .into_iter()
            .filter_map(|(key, value)| {
                let raw = match value {
                    Value::Object(mut typed) => typed.remove("value")?,
                    other => other,
                };
                let value = match raw {
                    Value::Bool(flag) => StateValue::Int(i64::from(flag)),
                    Value::Number(number) => StateValue::Int(number.as_i64()?),
                    Value::String(text) => StateValue::Text(text.into()),
                    _ => return None,
                };
                Some((key.into(), value))
            })
            .collect();
        Self(values)
    }

    pub(super) fn int(&self, key: &str) -> Option<i64> {
        match self.0.get(key)? {
            StateValue::Int(value) => Some(*value),
            StateValue::Text(_) => None,
        }
    }

    pub(super) fn text(&self, key: &str) -> Option<&str> {
        match self.0.get(key)? {
            StateValue::Text(value) => Some(value),
            StateValue::Int(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typed_and_plain_values() {
        let state = BlockState::parse(
            r#"{"facing_direction":{"type":"int","value":3},"minecraft:cardinal_direction":{"type":"string","value":"west"},"open_bit":true,"n":5}"#,
        );
        assert_eq!(state.int("facing_direction"), Some(3));
        assert_eq!(state.text("minecraft:cardinal_direction"), Some("west"));
        assert_eq!(state.int("open_bit"), Some(1));
        assert_eq!(state.int("n"), Some(5));
        assert_eq!(state.text("facing_direction"), None);
        assert_eq!(BlockState::parse("not json"), BlockState::default());
    }
}
