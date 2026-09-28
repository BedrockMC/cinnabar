//! Owned NetworkLittleEndian NBT tree for readers that need field values, not just bytes.

use std::collections::BTreeMap;

use crate::{BlockEntityNbt, MAX_NBT_DEPTH};

/// One decoded NBT value.
#[derive(Debug, Clone, PartialEq)]
pub enum NbtValue {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(Box<str>),
    List(Vec<NbtValue>),
    Compound(NbtCompound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// A compound with typed, lenient accessors: a missing or wrongly typed key reads as `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NbtCompound(BTreeMap<Box<str>, NbtValue>);

impl NbtCompound {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&NbtValue> {
        self.0.get(key)
    }

    /// Any integral tag widened to `i64`.
    #[must_use]
    pub fn integer(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            NbtValue::Byte(value) => Some(i64::from(*value)),
            NbtValue::Short(value) => Some(i64::from(*value)),
            NbtValue::Int(value) => Some(i64::from(*value)),
            NbtValue::Long(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.integer(key).map(|value| value != 0)
    }

    #[must_use]
    pub fn float(&self, key: &str) -> Option<f32> {
        match self.get(key)? {
            NbtValue::Float(value) => Some(*value),
            NbtValue::Double(value) => Some(*value as f32),
            other => match other {
                NbtValue::Byte(_) | NbtValue::Short(_) | NbtValue::Int(_) | NbtValue::Long(_) => {
                    self.integer(key).map(|value| value as f32)
                }
                _ => None,
            },
        }
    }

    #[must_use]
    pub fn string(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            NbtValue::String(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn compound(&self, key: &str) -> Option<&Self> {
        match self.get(key)? {
            NbtValue::Compound(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn list(&self, key: &str) -> Option<&[NbtValue]> {
        match self.get(key)? {
            NbtValue::List(value) => Some(value),
            _ => None,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &NbtValue)> {
        self.0.iter().map(|(key, value)| (key.as_ref(), value))
    }
}

impl BlockEntityNbt {
    /// Decodes the retained root compound; `None` when the bytes do not parse.
    #[must_use]
    pub fn parse(&self) -> Option<NbtCompound> {
        let mut reader = Cursor {
            input: self.bytes(),
            position: 0,
        };
        if reader.byte()? != 10 {
            return None;
        }
        let _root_name = reader.string()?;
        reader.compound(0)
    }
}

struct Cursor<'a> {
    input: &'a [u8],
    position: usize,
}

impl Cursor<'_> {
    fn take(&mut self, length: usize) -> Option<&[u8]> {
        let end = self.position.checked_add(length)?;
        let bytes = self.input.get(self.position..end)?;
        self.position = end;
        Some(bytes)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn var_u64(&mut self, max_bytes: usize) -> Option<u64> {
        let mut value = 0_u64;
        for index in 0..max_bytes {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }

    fn zigzag_i32(&mut self) -> Option<i32> {
        let value = self.var_u64(5)? as u32;
        Some(((value >> 1) as i32) ^ -((value & 1) as i32))
    }

    fn zigzag_i64(&mut self) -> Option<i64> {
        let value = self.var_u64(10)?;
        Some(((value >> 1) as i64) ^ -((value & 1) as i64))
    }

    fn length(&mut self) -> Option<usize> {
        usize::try_from(self.zigzag_i32()?).ok()
    }

    fn string(&mut self) -> Option<Box<str>> {
        let length = usize::try_from(self.var_u64(5)?).ok()?;
        Some(std::str::from_utf8(self.take(length)?).ok()?.into())
    }

    fn compound(&mut self, depth: usize) -> Option<NbtCompound> {
        if depth > MAX_NBT_DEPTH {
            return None;
        }
        let mut entries = BTreeMap::new();
        loop {
            let tag = self.byte()?;
            if tag == 0 {
                return Some(NbtCompound(entries));
            }
            let name = self.string()?;
            entries.insert(name, self.value(tag, depth)?);
        }
    }

    fn value(&mut self, tag: u8, depth: usize) -> Option<NbtValue> {
        Some(match tag {
            1 => NbtValue::Byte(self.byte()? as i8),
            2 => NbtValue::Short(i16::from_le_bytes(self.take(2)?.try_into().ok()?)),
            3 => NbtValue::Int(self.zigzag_i32()?),
            4 => NbtValue::Long(self.zigzag_i64()?),
            5 => NbtValue::Float(f32::from_le_bytes(self.take(4)?.try_into().ok()?)),
            6 => NbtValue::Double(f64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            7 => {
                let length = self.length()?;
                NbtValue::ByteArray(self.take(length)?.to_vec())
            }
            8 => NbtValue::String(self.string()?),
            9 => {
                let element = self.byte()?;
                let length = self.length()?;
                if depth + 1 > MAX_NBT_DEPTH || length > self.input.len() {
                    return None;
                }
                let mut items = Vec::with_capacity(length.min(64));
                for _ in 0..length {
                    items.push(self.value(element, depth + 1)?);
                }
                NbtValue::List(items)
            }
            10 => NbtValue::Compound(self.compound(depth + 1)?),
            11 => {
                let length = self.length()?;
                if length > self.input.len() {
                    return None;
                }
                let mut items = Vec::with_capacity(length.min(64));
                for _ in 0..length {
                    items.push(self.zigzag_i32()?);
                }
                NbtValue::IntArray(items)
            }
            12 => {
                let length = self.length()?;
                if length > self.input.len() {
                    return None;
                }
                let mut items = Vec::with_capacity(length.min(64));
                for _ in 0..length {
                    items.push(self.zigzag_i64()?);
                }
                NbtValue::LongArray(items)
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, text: &str) {
        out.push(text.len() as u8);
        out.extend_from_slice(text.as_bytes());
    }

    #[test]
    fn parses_nested_compound_with_mixed_tags() {
        let mut bytes = vec![10];
        string(&mut bytes, "");
        bytes.push(8);
        string(&mut bytes, "id");
        string(&mut bytes, "Sign");
        bytes.push(3);
        string(&mut bytes, "x");
        bytes.push(20); // zigzag(10)
        bytes.push(1);
        string(&mut bytes, "IsWaxed");
        bytes.push(1);
        bytes.push(10);
        string(&mut bytes, "FrontText");
        bytes.push(8);
        string(&mut bytes, "Text");
        string(&mut bytes, "hi");
        bytes.push(0);
        bytes.push(0);
        let (nbt, used) = BlockEntityNbt::decode_prefix(&bytes).unwrap();
        assert_eq!(used, bytes.len());
        let root = nbt.parse().unwrap();
        assert_eq!(root.string("id"), Some("Sign"));
        assert_eq!(root.integer("x"), Some(10));
        assert_eq!(root.boolean("IsWaxed"), Some(true));
        assert_eq!(
            root.compound("FrontText").unwrap().string("Text"),
            Some("hi")
        );
        assert_eq!(root.string("missing"), None);
    }
}
