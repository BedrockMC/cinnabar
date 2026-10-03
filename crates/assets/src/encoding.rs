/// Checks the complete carrier budget before appending a serialized field.
pub(crate) fn append_bounded(
    payload: &mut Vec<u8>,
    bytes: &[u8],
    limit: usize,
    overhead: usize,
) -> Option<()> {
    if payload
        .len()
        .checked_add(bytes.len())?
        .checked_add(overhead)?
        > limit
    {
        return None;
    }
    payload.extend_from_slice(bytes);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_oversized_carrier_fields_are_refused_before_allocation() {
        let mut payload = Vec::new();
        assert!(append_bounded(&mut payload, &[0; 17], 16, 4).is_none());
        assert_eq!(payload.capacity(), 0);
        assert!(append_bounded(&mut payload, &[0; 12], 16, 4).is_some());
        let capacity = payload.capacity();
        assert!(append_bounded(&mut payload, &[0], 16, 4).is_none());
        assert_eq!(payload.len(), 12);
        assert_eq!(payload.capacity(), capacity);
    }
}
