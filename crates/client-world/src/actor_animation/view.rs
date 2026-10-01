/// The view animation runs for: actors outside it hold their pose, since vanilla evaluates
/// pre-animation, animation and render controllers only for actors it renders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorAnimationView {
    /// Clip-space half-spaces `[a, b, c, d]`, inside where `a*x + b*y + c*z + d >= 0`.
    pub planes: [[f32; 4]; 6],
    pub camera: [f32; 3],
    /// Players farther than this from the camera hold their pose.
    pub player_distance: f32,
    /// Other actors farther than this from the camera on any axis hold their pose.
    pub entity_radius: f32,
}

impl ActorAnimationView {
    /// Whether the culling box of an actor at `feet` with model scale `scale` may be seen; the
    /// box matches the renderer's so nothing drawn is left unanimated.
    #[must_use]
    pub fn admits(&self, feet: [f32; 3], scale: f32, player: bool) -> bool {
        if feet.iter().any(|value| !value.is_finite()) {
            return true;
        }
        let offset: [f32; 3] = std::array::from_fn(|axis| feet[axis] - self.camera[axis]);
        if player {
            let head = [offset[0], offset[1] + 1.0, offset[2]];
            if head.iter().map(|value| value * value).sum::<f32>()
                > self.player_distance * self.player_distance
            {
                return false;
            }
        } else if offset.iter().any(|value| value.abs() > self.entity_radius) {
            return false;
        }
        let scale = if scale.is_finite() {
            scale.max(1.0)
        } else {
            1.0
        };
        let low = [feet[0] - 0.5 * scale, feet[1], feet[2] - 0.5 * scale];
        let high = [
            feet[0] + 0.5 * scale,
            feet[1] + 2.0 * scale,
            feet[2] + 0.5 * scale,
        ];
        self.planes.iter().all(|plane| {
            let corner: [f32; 3] = std::array::from_fn(|axis| {
                if plane[axis] >= 0.0 {
                    high[axis]
                } else {
                    low[axis]
                }
            });
            plane[0] * corner[0] + plane[1] * corner[1] + plane[2] * corner[2] + plane[3] >= 0.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ActorAnimationView;

    /// The half-space `x <= 10` alone, with generous distances.
    fn wall() -> ActorAnimationView {
        ActorAnimationView {
            planes: [[-1.0, 0.0, 0.0, 10.0]; 6],
            camera: [0.0; 3],
            player_distance: 100.0,
            entity_radius: 72.0,
        }
    }

    #[test]
    fn a_box_straddling_a_plane_is_admitted_and_one_past_it_is_not() {
        assert!(wall().admits([10.4, 0.0, 0.0], 1.0, false));
        assert!(!wall().admits([10.6, 0.0, 0.0], 1.0, false));
        // A scaled box reaches farther across the plane.
        assert!(wall().admits([11.5, 0.0, 0.0], 4.0, false));
    }

    #[test]
    fn entities_use_the_candidate_cube_and_players_the_distance() {
        let view = ActorAnimationView {
            planes: [[0.0, 0.0, 0.0, 1.0]; 6],
            ..wall()
        };
        assert!(!view.admits([0.0, 0.0, 73.0], 1.0, false));
        assert!(view.admits([0.0, 0.0, 73.0], 1.0, true));
        assert!(!view.admits([0.0, 0.0, 101.0], 1.0, true));
    }
}
