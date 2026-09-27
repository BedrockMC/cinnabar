use bevy::prelude::Resource;
use resource_pack::{LayeredPackView, PackAdmission};

pub(super) fn prepare_pack_application(
    handoff: protocol::ResourcePackHandoff,
) -> (
    PackAdmission,
    Option<std::sync::Arc<assets::ServerLangOverlay>>,
) {
    if handoff.is_empty() {
        return (PackAdmission::None, None);
    }
    let stack = resource_pack::validate_handoff(handoff);
    for rejection in stack.rejections() {
        bevy::log::warn!(
            stack_index = rejection.stack_index,
            reason = %rejection.reason,
            "server resource pack dropped"
        );
    }
    let view = LayeredPackView::new(std::sync::Arc::clone(&stack));
    let overlay = view.read("texts/en_US.lang").and_then(|bytes| {
        assets::ServerLangOverlay::read(bytes.len(), |output| {
            output.copy_from_slice(&bytes);
            true
        })
    });
    (PackAdmission::Validated(stack), overlay)
}

pub(super) fn install_server_language(
    runtime: &mut crate::ui_runtime::UiRuntime,
    generation: u64,
    overlay: Option<std::sync::Arc<assets::ServerLangOverlay>>,
    setup_succeeded: bool,
) {
    if runtime.session_id() == generation {
        runtime.set_server_lang(if setup_succeeded { overlay } else { None });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BootstrapGenerationDisposition {
    Expected,
    Stale,
    Unexpected,
}

pub(crate) const fn classify_bootstrap_generation(
    ui_generation: u64,
    world_generation: u64,
    incoming_generation: u64,
) -> BootstrapGenerationDisposition {
    let directly_next = ui_generation == world_generation
        && matches!(
            world_generation.checked_add(1),
            Some(expected) if expected == incoming_generation
        );
    let pending_ui_generation =
        incoming_generation == ui_generation && incoming_generation > world_generation;
    if directly_next || pending_ui_generation {
        BootstrapGenerationDisposition::Expected
    } else if incoming_generation <= world_generation || incoming_generation < ui_generation {
        BootstrapGenerationDisposition::Stale
    } else {
        BootstrapGenerationDisposition::Unexpected
    }
}

/// Generation-bound admission for the current session's optional pack stack.
/// This owns validated bytes independently of optional language application.
#[derive(Debug, Resource)]
pub(crate) struct ResourcePackAdmissionState {
    generation: u64,
    admission: PackAdmission,
}

impl Default for ResourcePackAdmissionState {
    fn default() -> Self {
        Self {
            generation: 0,
            admission: PackAdmission::None,
        }
    }
}

impl ResourcePackAdmissionState {
    /// Starts ownership for a pending generation and releases the prior stack.
    pub(crate) fn begin_generation(&mut self, generation: u64) -> bool {
        if generation <= self.generation {
            return false;
        }
        self.generation = generation;
        self.admission = PackAdmission::None;
        true
    }

    /// Publishes admission only for the pending/current or a newer generation.
    pub(crate) fn replace_for_generation(
        &mut self,
        generation: u64,
        admission: PackAdmission,
    ) -> bool {
        if generation < self.generation {
            return false;
        }
        self.generation = generation;
        self.admission = admission;
        true
    }

    /// Releases admission when the current network session terminates.
    pub(crate) fn clear_current(&mut self) {
        self.admission = PackAdmission::None;
    }

    #[cfg(test)]
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) const fn admission(&self) -> &PackAdmission {
        &self.admission
    }
}

#[cfg(test)]
mod tests {
    use resource_pack::{AdmissionError, PackAdmission};

    use super::ResourcePackAdmissionState;

    #[test]
    fn absent_or_rejected_application_preserves_optional_admission() {
        let (admission, overlay) =
            super::prepare_pack_application(protocol::ResourcePackHandoff::default());
        assert!(matches!(admission, PackAdmission::None));
        assert!(overlay.is_none());
        let pack = protocol::ResourcePackArchive::unencrypted(
            "11111111-2222-3333-4444-555555555555".parse().unwrap(),
            "1.2.3".into(),
            String::new(),
            vec![0; 32],
        );
        let (admission, overlay) =
            super::prepare_pack_application(protocol::ResourcePackHandoff::from_archives(vec![
                pack,
            ]));
        let PackAdmission::Validated(stack) = admission else {
            panic!("a dropped pack still yields an admitted stack");
        };
        assert!(stack.packs().is_empty());
        assert_eq!(
            stack.rejections()[0].reason,
            AdmissionError::InvalidZipFooter
        );
        assert!(overlay.is_none());
    }

    #[test]
    fn newer_generation_replaces_atomically_and_stale_results_are_ignored() {
        let mut state = ResourcePackAdmissionState::default();
        assert!(state.begin_generation(2));
        assert!(matches!(state.admission(), PackAdmission::None));
        let stack = resource_pack::validate_handoff(protocol::ResourcePackHandoff::default());
        assert!(state.replace_for_generation(2, PackAdmission::Validated(stack)));
        assert!(!state.replace_for_generation(1, PackAdmission::None));
        assert_eq!(state.generation(), 2);
        assert!(matches!(state.admission(), PackAdmission::Validated(_)));
        assert!(state.begin_generation(3));
        assert!(matches!(state.admission(), PackAdmission::None));
        assert!(!state.begin_generation(2));
        state.clear_current();
        assert_eq!(state.generation(), 3);
        assert!(matches!(state.admission(), PackAdmission::None));
    }
}
