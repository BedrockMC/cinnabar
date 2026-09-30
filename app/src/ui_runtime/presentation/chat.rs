use ui::UiPoint;

use super::UiPresentationRuntime;

impl UiPresentationRuntime {
    pub(crate) fn hit_test_leave_bed(&self, position: UiPoint, logical_size: [f32; 2]) -> bool {
        let expected = self.chat_hit_logical_size;
        expected.is_some_and(|size| size.map(f32::to_bits) == logical_size.map(f32::to_bits))
            && self
                .leave_bed_hit
                .is_some_and(|bounds| bounds.contains(position))
    }
}
