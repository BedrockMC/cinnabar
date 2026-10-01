//! What the client's controls keep between binds: each property bag, the
//! component state bindings last set, and every binding's schedule memory.
//! Controls are addressed by layout key.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{BuildHasherDefault, Hasher};

use serde_json::Value;

use crate::predicate::Scalar;
use crate::state::LayoutReport;

/// FNV-1a over a layout key, streamed so a child extends its parent's hash.
pub(super) fn key_hash(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The hash of the empty key, where every layout key starts.
pub(super) const KEY_ROOT: u64 = 0xcbf2_9ce4_8422_2325;

/// Hashes a map key that already is a key hash.
#[derive(Default)]
pub(super) struct Prehashed(u64);

impl Hasher for Prehashed {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0 = key_hash(self.0 ^ KEY_ROOT, bytes);
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

pub(super) type KeyMap<V> = HashMap<u64, V, BuildHasherDefault<Prehashed>>;

/// A screen's live binding state, kept by the caller across binds of the same
/// screen and dropped when the screen changes.
#[derive(Clone, Debug, Default)]
pub struct BindState {
    /// By layout-key hash.
    pub(super) controls: KeyMap<Retained>,
    /// Bag writes from widgets and components since the last bind.
    pub(super) published: KeyMap<BTreeMap<String, Scalar>>,
    /// The refresh count, which marks the controls each refresh built.
    pub(super) generation: u64,
}

/// One control's memory.
#[derive(Clone, Debug, Default)]
pub(super) struct Retained {
    pub(super) bag: BTreeMap<String, Scalar>,
    /// Literal properties bindings set on components.
    pub(super) native: BTreeMap<String, Value>,
    /// `once` bindings already applied, by index.
    pub(super) once: BTreeSet<usize>,
    /// Visibility each `visibility_changed` binding last applied at.
    pub(super) seen: BTreeMap<usize, bool>,
    /// Last source value each registered view observed.
    pub(super) views: BTreeMap<usize, Option<Scalar>>,
    /// The parent's key hash.
    pub(super) parent: u64,
    /// The refresh that last built this control.
    pub(super) generation: u64,
    /// Whether its subtree waited hidden then.
    pub(super) deferred: bool,
}

impl BindState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write `name` into the bag of the control at layout `key`, as a widget or
    /// component publishes state; views reading it update on the next bind.
    pub fn publish(&mut self, key: &str, name: &str, value: Scalar) {
        self.published
            .entry(key_hash(KEY_ROOT, key.as_bytes()))
            .or_default()
            .insert(name.to_owned(), value);
    }

    /// The bag value `name` of the control at layout `key` after the last bind.
    pub fn value(&self, key: &str, name: &str) -> Option<&Scalar> {
        let key = key_hash(KEY_ROOT, key.as_bytes());
        self.published
            .get(&key)
            .and_then(|values| values.get(name))
            .or_else(|| self.controls.get(&key)?.bag.get(name))
    }

    /// Publish each scroll view's end and scrollbar state from a layout, as
    /// `ScrollViewComponent` does; `true` when a published value changed.
    pub fn publish_scrolls(&mut self, report: &LayoutReport) -> bool {
        let mut changed = false;
        for (key, metrics) in &report.scrolls {
            let at_end = metrics.offset + 0.5 >= metrics.max_offset();
            let hit = at_end
                || matches!(
                    self.value(key, "#scrollbar_hit_bottom"),
                    Some(Scalar::Bool(true))
                );
            for (name, value) in [
                ("#scrolled_to_end", at_end),
                ("#scrollbar_hit_bottom", hit),
                ("#scroll_bar_visible", metrics.thumb.is_some()),
            ] {
                if self.value(key, name) != Some(&Scalar::Bool(value)) {
                    self.publish(key, name, Scalar::Bool(value));
                    changed = true;
                }
            }
        }
        changed
    }

    /// Whether a bind has run over this state yet.
    pub fn is_empty(&self) -> bool {
        self.controls.is_empty()
    }
}
