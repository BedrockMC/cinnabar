use std::sync::Arc;

use jolyne::GameData;

/// NBT nesting a block definition may use before it is treated as malformed.
const MAX_NBT_DEPTH: usize = 32;
/// States one custom block may contribute before it is treated as malformed.
const MAX_STATES_PER_BLOCK: u64 = 1 << 16;

/// One server-defined block from StartGame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomBlock {
    pub name: Arc<str>,
    /// Sequential palette states: the product of property and trait values.
    pub state_count: u32,
    /// False when the definition disables its collision box.
    pub collides: bool,
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomBlocks {
    pub blocks: Arc<[CustomBlock]>,
    pub skipped: usize,
}

impl CustomBlocks {
    #[must_use]
    pub fn from_game_data(game_data: &GameData) -> Self {
        let mut blocks = Vec::new();
        let mut skipped = 0;
        for property in &game_data.start_game.block_properties {
            match parse_definition(&property.block_definition.0) {
                Some((state_count, collides)) => blocks.push(CustomBlock {
                    name: Arc::from(property.block_name.as_str()),
                    state_count,
                    collides,
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

fn parse_definition(bytes: &[u8]) -> Option<(u32, bool)> {
    let mut reader = NbtReader { bytes, position: 0 };
    if reader.u8()? != TAG_COMPOUND {
        return None;
    }
    reader.string()?;
    let root = reader.payload(TAG_COMPOUND, 0)?;
    let mut states = 1_u64;
    for property in root.list("properties") {
        let values = property.list("enum").len().max(1);
        states = states.checked_mul(values as u64)?;
    }
    for name in root.list("traits").iter().flat_map(|trait_| {
        trait_
            .field("enabled_states")
            .map(Nbt::enabled_flags)
            .unwrap_or_default()
    }) {
        states = states.checked_mul(trait_state_values(&name))?;
    }
    if states > MAX_STATES_PER_BLOCK {
        return None;
    }
    let collides = match root
        .field("components")
        .and_then(|components| components.field("minecraft:collision_box"))
    {
        Some(Nbt::Byte(enabled)) => *enabled != 0,
        Some(compound @ Nbt::Compound(_)) => {
            !matches!(compound.field("enabled"), Some(Nbt::Byte(0)))
        }
        _ => true,
    };
    Some((u32::try_from(states).ok()?, collides))
}

/// Values a placement trait state contributes; unknown states contribute one.
fn trait_state_values(state: &str) -> u64 {
    match state {
        "cardinal_direction" => 4,
        "facing_direction" | "block_face" => 6,
        "vertical_half" => 2,
        _ => 1,
    }
}

const TAG_COMPOUND: u8 = 10;

#[derive(Debug)]
enum Nbt {
    Byte(i8),
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

    fn enabled_flags(&self) -> Vec<String> {
        match self {
            Self::Compound(fields) => fields
                .iter()
                .filter(|(_, value)| matches!(value, Self::Byte(flag) if *flag != 0))
                .map(|(key, _)| key.clone())
                .collect(),
            _ => Vec::new(),
        }
    }
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
            2 => self.take(2).map(|_| Nbt::Other)?,
            3 => self.var_u64(5).map(|_| Nbt::Other)?,
            4 => self.var_u64(10).map(|_| Nbt::Other)?,
            5 => self.take(4).map(|_| Nbt::Other)?,
            6 => self.take(8).map(|_| Nbt::Other)?,
            7 => {
                let length = self.length()?;
                self.take(length).map(|_| Nbt::Other)?
            }
            8 => self.string().map(|_| Nbt::Other)?,
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
        assert_eq!(parse_definition(&nbt), Some((2 * 3 * 4, false)));
    }

    #[test]
    fn truncated_definition_is_rejected() {
        assert_eq!(parse_definition(&[10, 0, 9]), None);
    }

    #[test]
    fn sort_key_is_fnv1_64_of_the_name() {
        assert_eq!(block_name_sort_key(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(block_name_sort_key("a"), 0xaf63_bd4c_8601_b7be);
    }
}
