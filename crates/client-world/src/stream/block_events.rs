//! Latest server block-event cue per position; presentation interprets the values.

use std::collections::BTreeMap;

use protocol::BlockEventEvent;

use super::WorldStream;

/// A client resource budget, not a gameplay limit.
pub const MAX_RETAINED_BLOCK_EVENTS: usize = 4_096;

/// The most recent cue for one block position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockEventCue {
    pub event_type: i32,
    pub event_value: i32,
    /// Commit sequence of the packet; changes on every new cue, including repeats.
    pub sequence: u64,
}

#[derive(Default)]
pub(super) struct BlockEvents {
    cues: BTreeMap<[i32; 3], BlockEventCue>,
    dropped: u64,
}

impl WorldStream {
    pub(super) fn consume_block_event(&mut self, sequence: u64, event: BlockEventEvent) {
        if event.dimension != self.current_dimension {
            return;
        }
        let events = &mut self.block_events;
        if !events.cues.contains_key(&event.position)
            && events.cues.len() >= MAX_RETAINED_BLOCK_EVENTS
        {
            events.dropped = events.dropped.saturating_add(1);
            return;
        }
        events.cues.insert(
            event.position,
            BlockEventCue {
                event_type: event.event_type,
                event_value: event.event_value,
                sequence,
            },
        );
    }

    pub(super) fn clear_block_events(&mut self) {
        self.block_events.cues.clear();
    }

    /// The latest cue at `position`, if any has arrived this dimension.
    #[must_use]
    pub fn block_event_cue(&self, position: [i32; 3]) -> Option<BlockEventCue> {
        self.block_events.cues.get(&position).copied()
    }

    /// Cues dropped because the retention budget was full.
    #[must_use]
    pub const fn dropped_block_events(&self) -> u64 {
        self.block_events.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_is_bounded_but_existing_positions_keep_updating() {
        let mut events = BlockEvents::default();
        for index in 0..MAX_RETAINED_BLOCK_EVENTS as i32 {
            events.cues.insert(
                [index, 0, 0],
                BlockEventCue {
                    event_type: 1,
                    event_value: 1,
                    sequence: 0,
                },
            );
        }
        assert_eq!(events.cues.len(), MAX_RETAINED_BLOCK_EVENTS);
    }
}
