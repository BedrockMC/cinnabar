/// Largest walk speed prediction can simulate: steady sprint velocity reaches
/// about 2.9x the speed, which must stay inside the collision query extent.
const MAX_SIMULABLE_MOVEMENT_SPEED: f64 = sim::MAX_COLLISION_QUERY_EXTENT / 4.0;

#[derive(Debug, Default)]
pub struct LocalMovementSpeedAuthority {
    session_id: u64,
    dimension: i32,
    last_sequence: Option<u64>,
    current: Option<f64>,
}

impl LocalMovementSpeedAuthority {
    pub fn begin_session(&mut self, session_id: u64, dimension: i32) {
        self.session_id = session_id;
        self.dimension = dimension;
        self.last_sequence = None;
        self.current = None;
    }

    pub fn replace_dimension(&mut self, session_id: u64, dimension: i32) {
        if session_id != self.session_id {
            return;
        }
        self.dimension = dimension;
        self.last_sequence = None;
        self.current = None;
    }

    pub fn apply(&mut self, session_id: u64, sequence: u64, dimension: i32, current: f64) -> bool {
        if session_id != self.session_id
            || dimension != self.dimension
            || self.last_sequence.is_some_and(|last| sequence <= last)
        {
            return false;
        }
        self.last_sequence = Some(sequence);
        if !(0.0..=MAX_SIMULABLE_MOVEMENT_SPEED).contains(&current) {
            super::diagnostics::note_skipped_authority("movement_speed", current);
            return false;
        }
        self.current = Some(current);
        true
    }

    pub const fn current(&self) -> Option<f64> {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::LocalMovementSpeedAuthority;

    #[test]
    fn authority_obeys_session_fifo_dimension_and_replacement_ordering() {
        let mut authority = LocalMovementSpeedAuthority::default();
        authority.begin_session(7, 0);
        assert!(authority.apply(7, 2, 0, 0.25));
        assert!(!authority.apply(6, 3, 0, 0.5));
        assert!(!authority.apply(7, 1, 0, 0.5));
        assert!(!authority.apply(7, 3, 1, 0.5));
        assert_eq!(authority.current(), Some(0.25));

        authority.replace_dimension(7, 1);
        assert_eq!(authority.current(), None);
        assert!(!authority.apply(7, 1, 0, 0.75));
        assert!(authority.apply(7, 1, 1, 0.0));
        assert_eq!(authority.current(), Some(0.0));

        authority.begin_session(8, -1);
        assert_eq!(authority.current(), None);
        assert!(!authority.apply(7, 2, -1, 1.0));
        assert!(authority.apply(8, 1, -1, 0.1));
    }

    #[test]
    fn invalid_updates_are_consumed_without_overwriting_last_valid_authority() {
        let mut authority = LocalMovementSpeedAuthority::default();
        authority.begin_session(1, 0);
        assert!(authority.apply(1, 1, 0, 0.2));
        for (sequence, value) in [(2, f64::NAN), (3, f64::INFINITY), (4, -0.1), (5, 1.0e6)] {
            assert!(!authority.apply(1, sequence, 0, value));
            assert_eq!(authority.current(), Some(0.2));
        }
        assert!(!authority.apply(1, 5, 0, 0.9));
        assert_eq!(authority.current(), Some(0.2));
    }
}
