use assets::EntityRenderMaterialState;

use super::ActorMaterial;

impl ActorMaterial {
    /// Shader kind and independently admitted raster states in the shared instance word.
    pub fn gpu_word(self) -> u32 {
        self.kind.word(self.state)
    }
}

pub(crate) fn state(word: u32) -> Option<EntityRenderMaterialState> {
    EntityRenderMaterialState::from_word(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_actor_states_roundtrip_without_changing_the_shader_kind() {
        for bits in 0..16 {
            let expected = EntityRenderMaterialState {
                alpha_test: bits & 1 != 0,
                cull: bits & 2 != 0,
                blend: bits & 4 != 0,
                depth_write: bits & 8 != 0,
            };
            let material = ActorMaterial {
                kind: assets::EntityRenderMaterial::Default,
                state: Some(expected),
                ..Default::default()
            };
            let word = material.gpu_word();
            assert_eq!(state(word), Some(expected));
            assert_eq!(
                word & EntityRenderMaterialState::KIND_MASK,
                material.kind as u32
            );
        }
        assert_eq!(
            ActorMaterial::default().gpu_word(),
            assets::EntityRenderMaterial::Default as u32
        );
        assert_eq!(state(ActorMaterial::default().gpu_word()), None);
    }
}
