use std::sync::Arc;

use jolyne::GameData;

/// NBT nesting a block definition may use before it is treated as malformed.
const MAX_NBT_DEPTH: usize = 32;
/// States one custom block may contribute before it is treated as malformed.
const MAX_STATES_PER_BLOCK: u64 = 1 << 16;

/// Permutations one custom block may define before extras are ignored.
const MAX_PERMUTATIONS: usize = 1024;
/// Material instances one component set may define before extras are ignored.
const MAX_MATERIAL_INSTANCES: usize = 64;

/// One server-defined block from StartGame.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomBlock {
    pub name: Arc<str>,
    /// Sequential palette states: the product of property and trait values.
    pub state_count: u32,
    /// False when the definition disables its collision box.
    pub collides: bool,
    pub visual: Arc<CustomBlockVisuals>,
}

/// The render-relevant parts of a custom block definition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomBlockVisuals {
    pub base: CustomVisualComponents,
    /// In definition order; later matching permutations override earlier ones.
    pub permutations: Box<[CustomPermutation]>,
    /// Block states in definition order, properties first, then trait states.
    pub state_axes: Box<[CustomStateAxis]>,
}

/// Visual components present in one component set; `None` means absent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomVisualComponents {
    pub geometry: Option<Arc<str>>,
    pub materials: Option<Box<[CustomMaterialInstance]>>,
    pub transformation: Option<CustomTransformation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomMaterialInstance {
    /// `*`, a face name, or a named instance a geometry face refers to.
    pub name: Arc<str>,
    pub texture: Arc<str>,
    pub render_method: Option<Arc<str>>,
}

/// Rotation in quarter turns about each axis, then scale and translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CustomTransformation {
    pub rotation: [i32; 3],
    pub scale: [f32; 3],
    pub translation: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct CustomPermutation {
    pub condition: Arc<str>,
    pub components: CustomVisualComponents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomStateAxis {
    pub name: Arc<str>,
    pub values: Box<[CustomStateValue]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomStateValue {
    String(Arc<str>),
    Int(i64),
    Bool(bool),
}

impl CustomBlock {
    /// Vanilla orders the sequential block palette by FNV-1 64 of the name, then the name.
    #[must_use]
    pub fn sort_key(&self) -> u64 {
        block_name_sort_key(&self.name)
    }
}

/// Returns vanilla's sequential palette sort key for a block name.
#[must_use]
pub fn block_name_sort_key(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        hash.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(byte)
    })
}

/// StartGame custom blocks in sequential palette order; malformed definitions are skipped.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomBlocks {
    pub blocks: Arc<[CustomBlock]>,
    pub skipped: usize,
}

impl CustomBlocks {
    #[must_use]
    pub fn from_game_data(game_data: &GameData) -> Self {
        Self::from_definitions(
            game_data
                .start_game
                .block_properties
                .iter()
                .map(|property| {
                    (
                        property.block_name.as_str(),
                        property.block_definition.0.as_ref(),
                    )
                }),
        )
    }

    /// Parses `(name, network NBT)` block definitions as StartGame carries them.
    #[must_use]
    pub fn from_definitions<'a>(
        definitions: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> Self {
        let mut blocks = Vec::new();
        let mut skipped = 0;
        for (name, bytes) in definitions {
            match parse_definition(bytes) {
                Some(definition) => blocks.push(CustomBlock {
                    name: Arc::from(name),
                    state_count: definition.state_count,
                    collides: definition.collides,
                    visual: Arc::new(definition.visual),
                }),
                None => skipped += 1,
            }
        }
        blocks.sort_by(|left, right| {
            (left.sort_key(), &left.name).cmp(&(right.sort_key(), &right.name))
        });
        Self {
            blocks: blocks.into(),
            skipped,
        }
    }

    #[must_use]
    pub fn total_states(&self) -> u32 {
        self.blocks.iter().fold(0_u32, |total, block| {
            total.saturating_add(block.state_count)
        })
    }
}

struct Definition {
    state_count: u32,
    collides: bool,
    visual: CustomBlockVisuals,
}

fn parse_definition(bytes: &[u8]) -> Option<Definition> {
    let mut reader = NbtReader { bytes, position: 0 };
    if reader.u8()? != TAG_COMPOUND {
        return None;
    }
    reader.string()?;
    let root = reader.payload(TAG_COMPOUND, 0)?;
    let mut states = 1_u64;
    let mut state_axes = Vec::new();
    for property in root.list("properties") {
        let values = property.list("enum");
        states = states.checked_mul(values.len().max(1) as u64)?;
        if let Some(Nbt::String(name)) = property.field("name") {
            let values = values.iter().filter_map(Nbt::state_value).collect();
            state_axes.push(CustomStateAxis {
                name: name.as_str().into(),
                values,
            });
        }
    }
    for name in root.list("traits").iter().flat_map(|trait_| {
        trait_
            .field("enabled_states")
            .map(Nbt::enabled_flags)
            .unwrap_or_default()
    }) {
        states = states.checked_mul(trait_state_values(&name))?;
        if let Some(values) = trait_state_names(&name) {
            state_axes.push(CustomStateAxis {
                name: format!("minecraft:{name}").into(),
                values: values
                    .iter()
                    .map(|value| CustomStateValue::String((*value).into()))
                    .collect(),
            });
        }
    }
    if states > MAX_STATES_PER_BLOCK {
        return None;
    }
    let components = root.field("components");
    let collides =
        match components.and_then(|components| components.field("minecraft:collision_box")) {
            Some(Nbt::Byte(enabled)) => *enabled != 0,
            Some(compound @ Nbt::Compound(_)) => {
                !matches!(compound.field("enabled"), Some(Nbt::Byte(0)))
            }
            _ => true,
        };
    let permutations = root
        .list("permutations")
        .iter()
        .take(MAX_PERMUTATIONS)
        .filter_map(|permutation| {
            let Some(Nbt::String(condition)) = permutation.field("condition") else {
                return None;
            };
            Some(CustomPermutation {
                condition: condition.as_str().into(),
                components: visual_components(permutation.field("components")),
            })
        })
        .collect();
    Some(Definition {
        state_count: u32::try_from(states).ok()?,
        collides,
        visual: CustomBlockVisuals {
            base: visual_components(components),
            permutations,
            state_axes: state_axes.into_boxed_slice(),
        },
    })
}

/// Reads geometry, material instances, and transformation; odd values are
/// treated as absent so the block keeps its other visuals.
fn visual_components(components: Option<&Nbt>) -> CustomVisualComponents {
    let Some(components) = components else {
        return CustomVisualComponents::default();
    };
    let geometry = match components.field("minecraft:geometry") {
        Some(Nbt::String(identifier)) => Some(identifier.as_str().into()),
        Some(compound) => match compound.field("identifier") {
            Some(Nbt::String(identifier)) => Some(identifier.as_str().into()),
            _ => None,
        },
        None => None,
    };
    let materials = components
        .field("minecraft:material_instances")
        .and_then(|instances| match instances.field("materials") {
            Some(Nbt::Compound(fields)) => Some(fields),
            _ => None,
        })
        .map(|fields| {
            fields
                .iter()
                .take(MAX_MATERIAL_INSTANCES)
                .filter_map(|(name, material)| {
                    let Some(Nbt::String(texture)) = material.field("texture") else {
                        return None;
                    };
                    let render_method = match material.field("render_method") {
                        Some(Nbt::String(method)) => Some(method.as_str().into()),
                        _ => None,
                    };
                    Some(CustomMaterialInstance {
                        name: name.as_str().into(),
                        texture: texture.as_str().into(),
                        render_method,
                    })
                })
                .collect()
        });
    let transformation = components
        .field("minecraft:transformation")
        .map(|transform| {
            let number = |key: &str, default: f64| {
                transform
                    .field(key)
                    .and_then(Nbt::number)
                    .unwrap_or(default)
            };
            let quarter = |key: &str| (number(key, 0.0).round() as i64).rem_euclid(4) as i32;
            CustomTransformation {
                rotation: [quarter("RX"), quarter("RY"), quarter("RZ")],
                scale: [
                    number("SX", 1.0) as f32,
                    number("SY", 1.0) as f32,
                    number("SZ", 1.0) as f32,
                ],
                translation: [
                    number("TX", 0.0) as f32,
                    number("TY", 0.0) as f32,
                    number("TZ", 0.0) as f32,
                ],
            }
        })
        .filter(|transform| {
            transform
                .scale
                .iter()
                .chain(&transform.translation)
                .all(|value| value.is_finite())
        });
    CustomVisualComponents {
        geometry,
        materials,
        transformation,
    }
}

/// Values a placement trait state contributes; unknown states contribute one.
fn trait_state_values(state: &str) -> u64 {
    trait_state_names(state).map_or(1, |values| values.len() as u64)
}

/// Trait state values in the order vanilla enumerates the same block states
/// (public canonical block-state data).
fn trait_state_names(state: &str) -> Option<&'static [&'static str]> {
    match state {
        "cardinal_direction" => Some(&["south", "west", "north", "east"]),
        "facing_direction" | "block_face" => {
            Some(&["down", "up", "north", "south", "west", "east"])
        }
        "vertical_half" => Some(&["bottom", "top"]),
        _ => None,
    }
}

const TAG_COMPOUND: u8 = 10;

#[derive(Debug)]
enum Nbt {
    Byte(i8),
    Int(i64),
    Float(f64),
    String(String),
    Other,
    List(Vec<Nbt>),
    Compound(Vec<(String, Nbt)>),
}

impl Nbt {
    fn field(&self, name: &str) -> Option<&Nbt> {
        match self {
            Self::Compound(fields) => fields
                .iter()
                .find_map(|(key, value)| (key == name).then_some(value)),
            _ => None,
        }
    }

    fn list(&self, name: &str) -> &[Nbt] {
        match self.field(name) {
            Some(Self::List(items)) => items,
            _ => &[],
        }
    }

    fn number(&self) -> Option<f64> {
        match self {
            Self::Byte(value) => Some(f64::from(*value)),
            Self::Int(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    fn state_value(&self) -> Option<CustomStateValue> {
        match self {
            Self::String(value) => Some(CustomStateValue::String(value.as_str().into())),
            Self::Byte(value) => Some(CustomStateValue::Bool(*value != 0)),
            Self::Int(value) => Some(CustomStateValue::Int(*value)),
            _ => None,
        }
    }

    fn enabled_flags(&self) -> Vec<String> {
        match self {
            Self::Compound(fields) => fields
                .iter()
                .filter(|(_, value)| value.number().is_some_and(|flag| flag != 0.0))
                .map(|(key, _)| key.clone())
                .collect(),
            _ => Vec::new(),
        }
    }
}

fn zigzag(raw: u64) -> i64 {
    ((raw >> 1) as i64) ^ -((raw & 1) as i64)
}

/// Network little-endian NBT: VarInt lengths and zigzag VarInt ints/longs.
struct NbtReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl NbtReader<'_> {
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let bytes = self
            .bytes
            .get(self.position..self.position.checked_add(count)?)?;
        self.position += count;
        Some(bytes)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn var_u64(&mut self, max_bytes: usize) -> Option<u64> {
        let mut value = 0_u64;
        for index in 0..max_bytes {
            let byte = self.u8()?;
            value |= u64::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn length(&mut self) -> Option<usize> {
        let raw = self.var_u64(5)? as u32;
        let value = ((raw >> 1) as i32) ^ -((raw & 1) as i32);
        let length = usize::try_from(value).ok()?;
        (length <= self.bytes.len() - self.position).then_some(length)
    }

    fn string(&mut self) -> Option<String> {
        let length = usize::try_from(self.var_u64(5)?).ok()?;
        String::from_utf8(self.take(length)?.to_vec()).ok()
    }

    fn payload(&mut self, tag: u8, depth: usize) -> Option<Nbt> {
        if depth > MAX_NBT_DEPTH {
            return None;
        }
        Some(match tag {
            1 => Nbt::Byte(self.u8()? as i8),
            2 => {
                let bytes = self.take(2)?;
                Nbt::Int(i64::from(i16::from_le_bytes([bytes[0], bytes[1]])))
            }
            3 => Nbt::Int(zigzag(self.var_u64(5)?)),
            4 => Nbt::Int(zigzag(self.var_u64(10)?)),
            5 => Nbt::Float(f64::from(f32::from_le_bytes(
                self.take(4)?.try_into().ok()?,
            ))),
            6 => Nbt::Float(f64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            7 => {
                let length = self.length()?;
                self.take(length).map(|_| Nbt::Other)?
            }
            8 => Nbt::String(self.string()?),
            9 => {
                let element = self.u8()?;
                let length = self.length()?;
                let items = (0..length)
                    .map(|_| self.payload(element, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                Nbt::List(items)
            }
            TAG_COMPOUND => {
                let mut fields = Vec::new();
                loop {
                    let child = self.u8()?;
                    if child == 0 {
                        break Nbt::Compound(fields);
                    }
                    let name = self.string()?;
                    fields.push((name, self.payload(child, depth + 1)?));
                }
            }
            11 => {
                let length = self.length()?;
                for _ in 0..length {
                    self.var_u64(5)?;
                }
                Nbt::Other
            }
            12 => {
                let length = self.length()?;
                for _ in 0..length {
                    self.var_u64(10)?;
                }
                Nbt::Other
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{block_name_sort_key, parse_definition};

    fn string(value: &str) -> Vec<u8> {
        let mut bytes = vec![value.len() as u8];
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    fn named(tag: u8, name: &str) -> Vec<u8> {
        let mut bytes = vec![tag];
        bytes.extend(string(name));
        bytes
    }

    #[test]
    fn placement_trait_and_enum_properties_multiply_states() {
        let mut nbt = named(10, "");
        nbt.extend(named(9, "properties"));
        nbt.extend([10, 4]);
        for values in [2_u8, 3] {
            nbt.extend(named(9, "enum"));
            nbt.extend([8, values * 2]);
            for index in 0..values {
                nbt.extend(string(&index.to_string()));
            }
            nbt.push(0);
        }
        nbt.extend(named(9, "traits"));
        nbt.extend([10, 2]);
        nbt.extend(named(10, "enabled_states"));
        nbt.extend(named(1, "cardinal_direction"));
        nbt.extend([1, 0, 0]);
        nbt.extend(named(10, "components"));
        nbt.extend(named(1, "minecraft:collision_box"));
        nbt.extend([0, 0, 0]);
        let definition = parse_definition(&nbt).expect("definition");
        assert_eq!(
            (definition.state_count, definition.collides),
            (2 * 3 * 4, false)
        );
        let axes = &definition.visual.state_axes;
        assert_eq!(axes.len(), 1, "unnamed properties carry no axis");
        assert_eq!(axes[0].name.as_ref(), "minecraft:cardinal_direction");
        assert_eq!(
            axes[0].values[0],
            super::CustomStateValue::String("south".into())
        );
    }

    fn string_field(name: &str, value: &str) -> Vec<u8> {
        let mut bytes = named(8, name);
        bytes.extend(string(value));
        bytes
    }

    #[test]
    fn visual_components_and_permutations_are_retained() {
        let mut nbt = named(10, "");
        nbt.extend(named(10, "components"));
        nbt.extend(named(10, "minecraft:geometry"));
        nbt.extend(string_field("identifier", "geometry.ore"));
        nbt.push(0);
        nbt.extend(named(10, "minecraft:material_instances"));
        nbt.extend(named(10, "materials"));
        nbt.extend(named(10, "*"));
        nbt.extend(string_field("texture", "ore_top"));
        nbt.extend([0, 0, 0]);
        nbt.push(0);
        nbt.extend(named(9, "permutations"));
        nbt.extend([10, 2]);
        nbt.extend(string_field("condition", "q.block_state('x') == 'y'"));
        nbt.extend(named(10, "components"));
        nbt.extend(named(10, "minecraft:transformation"));
        nbt.extend(named(3, "RY"));
        nbt.push(4);
        nbt.extend(named(5, "SX"));
        nbt.extend(2.0_f32.to_le_bytes());
        nbt.extend([0, 0, 0]);
        nbt.push(0);
        let visual = parse_definition(&nbt).expect("definition").visual;
        assert_eq!(visual.base.geometry.as_deref(), Some("geometry.ore"));
        let materials = visual.base.materials.as_deref().expect("materials");
        assert_eq!(
            (materials[0].name.as_ref(), materials[0].texture.as_ref()),
            ("*", "ore_top")
        );
        let permutation = &visual.permutations[0];
        assert_eq!(permutation.condition.as_ref(), "q.block_state('x') == 'y'");
        let transform = permutation
            .components
            .transformation
            .expect("transformation");
        assert_eq!(
            transform.rotation,
            [0, 2, 0],
            "zigzag 4 is two quarter turns"
        );
        assert_eq!(transform.scale, [2.0, 1.0, 1.0]);
    }

    #[test]
    fn truncated_definition_is_rejected() {
        assert!(parse_definition(&[10, 0, 9]).is_none());
    }

    #[test]
    fn sort_key_is_fnv1_64_of_the_name() {
        assert_eq!(block_name_sort_key(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(block_name_sort_key("a"), 0xaf63_bd4c_8601_b7be);
    }
}
