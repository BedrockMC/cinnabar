/// Admits non-player actors inside vanilla's camera-centred candidate cube, including its edge.
/// R:LevelRendererCamera:14149–14156 and R:ActorRenderCandidates:32–67 gather this box.
pub(crate) fn within_actor_candidate_cube(position: [f32; 3], camera: [f32; 3]) -> bool {
    !(0..3)
        .any(|axis| (position[axis] - camera[axis]).abs() > render::ACTOR_CANDIDATE_RADIUS_BLOCKS)
}
