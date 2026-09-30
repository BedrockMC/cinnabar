//! Texture sidecar metadata: the `textures/ui/<name>.json` companion that gives a
//! sprite its native `base_size` and, when present, its nine-slice insets. Parsed
//! here so both layout (natural size) and emit (nine-slice split) share one model.
//! The sidecar bytes are Mojang DATA read from `.local`; nothing is committed.

use serde_json::Value;

/// Nine-slice insets in source pixels: the fixed border widths held to a 1:1 scale.
/// A zero inset collapses that border, letting the neighbouring region stretch to
/// the edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NineSlice {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// A sprite's native dimensions plus any nine-slice split.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureMeta {
    pub base_size: [f64; 2],
    pub nineslice: Option<NineSlice>,
}

/// Parse a sidecar JSON value. Returns `None` only when `base_size` is absent or
/// malformed, since without it neither natural sizing nor nine-slice can proceed.
pub fn parse_texture_meta(value: &Value) -> Option<TextureMeta> {
    let object = value.as_object()?;
    let base_size = read_pair(object.get("base_size")?)?;
    let nineslice = object.get("nineslice_size").and_then(parse_nineslice);
    Some(TextureMeta {
        base_size,
        nineslice,
    })
}

fn parse_nineslice(value: &Value) -> Option<NineSlice> {
    match value {
        Value::Number(number) => {
            let inset = number.as_f64()?;
            Some(NineSlice {
                left: inset,
                top: inset,
                right: inset,
                bottom: inset,
            })
        }
        Value::Array(items) if items.len() == 4 => {
            let read = |index: usize| items[index].as_f64();
            Some(NineSlice {
                left: read(0)?,
                top: read(1)?,
                right: read(2)?,
                bottom: read(3)?,
            })
        }
        _ => None,
    }
}

fn read_pair(value: &Value) -> Option<[f64; 2]> {
    let items = value.as_array()?;
    if items.len() != 2 {
        return None;
    }
    Some([items[0].as_f64()?, items[1].as_f64()?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalar_nineslice_expands_to_four_equal_insets() {
        let meta =
            parse_texture_meta(&json!({ "nineslice_size": 4, "base_size": [16, 16] })).unwrap();
        assert_eq!(meta.base_size, [16.0, 16.0]);
        assert_eq!(
            meta.nineslice,
            Some(NineSlice {
                left: 4.0,
                top: 4.0,
                right: 4.0,
                bottom: 4.0,
            })
        );
    }

    #[test]
    fn array_nineslice_keeps_per_edge_insets() {
        let meta =
            parse_texture_meta(&json!({ "nineslice_size": [8, 23, 8, 8], "base_size": [18, 33] }))
                .unwrap();
        assert_eq!(
            meta.nineslice,
            Some(NineSlice {
                left: 8.0,
                top: 23.0,
                right: 8.0,
                bottom: 8.0,
            })
        );
    }

    #[test]
    fn plain_sprite_has_base_size_without_nineslice() {
        let meta = parse_texture_meta(&json!({ "base_size": [64, 64] })).unwrap();
        assert!(meta.nineslice.is_none());
    }

    #[test]
    fn missing_base_size_is_rejected() {
        assert!(parse_texture_meta(&json!({ "nineslice_size": 4 })).is_none());
    }
}
