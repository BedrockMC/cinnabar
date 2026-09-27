//! Static neutral empty-hand adapter. Missing animation observers are capability
//! gaps, not evidence that the player is in a source-qualified idle state.
use crate::{camera::FlyCamera, runtime::world::ClientWorld, ui_runtime::UiRuntime};
use bevy::{
    camera::{Camera, RenderTarget},
    ecs::system::SystemParam,
    prelude::*,
    render::view::Hdr,
    window::WindowRef,
};
use render::{
    ViewmodelCompletionGate, ViewmodelGeometry, ViewmodelMode, ViewmodelScene, ViewmodelSkin,
    ViewmodelToken,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HandFallback {
    Hidden,
    Ownership,
    ItemsUnknownOrHeld,
    Skin,
    Geometry,
    View,
    KnownActive,
}
#[derive(Default, Debug)]
pub(crate) struct HandStats {
    pub(crate) mode: Option<ViewmodelMode>,
    pub(crate) fallback: Option<HandFallback>,
    pub(crate) neutral_eligible_frames: u64,
    // Main-world fallback requests; not rendered/presented frame counters.
    pub(crate) cpu_fallback_requested_frames: u64,
    pub(crate) skin_validations: u64,
    pub(crate) gpu_rejections: u64,
    // Exact claims of this mode, never inferred values of missing observations.
    pub(crate) animation_parity_unavailable: bool,
    pub(crate) lighting_parity_unavailable: bool,
    pub(crate) avatar_model_identity_unavailable: bool,
}
#[derive(Default, Resource)]
pub(crate) struct HandAdapter {
    raw_skin: Option<Arc<[u8]>>,
    skin: Option<ViewmodelSkin>,
    skin_identity: [u8; 32],
    revision: u64,
    revision_exhausted: bool,
    pub(crate) stats: HandStats,
}
impl HandAdapter {
    fn skin(&mut self, raw: &protocol::StandardSkin) -> Option<ViewmodelSkin> {
        if raw.width != 64
            || raw.height != 64
            || raw.rgba8.len() != 64 * 64 * 4
            || self.revision_exhausted
        {
            return None;
        }
        if self
            .raw_skin
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &raw.rgba8))
        {
            self.raw_skin = Some(Arc::clone(&raw.rgba8));
            self.skin_identity = Sha256::digest(&raw.rgba8).into();
            self.skin = ViewmodelSkin::new(Arc::clone(&raw.rgba8), self.skin_identity);
            self.stats.skin_validations = self.stats.skin_validations.saturating_add(1);
            if let Some(next) = self.revision.checked_add(1) {
                self.revision = next;
            } else {
                self.revision_exhausted = true;
                self.skin = None;
            }
        }
        self.skin.clone()
    }
}

type ViewmodelCameras<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Camera,
        &'static RenderTarget,
        &'static Msaa,
        Option<&'static Hdr>,
    ),
    With<FlyCamera>,
>;

#[derive(SystemParam)]
pub(crate) struct ViewmodelPublish<'w, 's> {
    scene: Option<ResMut<'w, ViewmodelScene>>,
    gate: Option<Res<'w, ViewmodelCompletionGate>>,
    adapter: Option<ResMut<'w, HandAdapter>>,
    geometry: Option<Res<'w, ViewmodelGeometry>>,
    cameras: ViewmodelCameras<'w, 's>,
}
impl ViewmodelPublish<'_, '_> {
    pub(crate) fn bind_cpu_fallback(
        &mut self,
        input: &render::UiRenderInput,
        icon: Option<crate::ui_runtime::presentation::IconRef>,
    ) {
        if let (Some(scene), Some(gate), Some(icon)) = (&mut self.scene, &self.gate, icon) {
            scene.bind_cpu_fallback(input, u32::from(icon.page), icon.uv, gate);
        } else {
            self.clear();
        }
    }
    pub(crate) fn clear(&mut self) {
        if let (Some(scene), Some(gate)) = (&mut self.scene, &self.gate) {
            scene.clear(gate);
        }
        if let Some(adapter) = &mut self.adapter {
            adapter.stats.mode = None;
            adapter.stats.fallback = Some(HandFallback::View);
            adapter.stats.animation_parity_unavailable = false;
            adapter.stats.lighting_parity_unavailable = false;
            adapter.stats.avatar_model_identity_unavailable = false;
        }
    }
    pub(crate) fn observe(
        &mut self,
        runtime: &UiRuntime,
        world: &ClientWorld,
        first_person: bool,
        hidden: bool,
        viewport: [u32; 2],
    ) -> bool {
        // UI-only and headless clients retain their existing CPU path without
        // installing the optional GPU hand resources.
        if self.adapter.is_none() || self.scene.is_none() || self.gate.is_none() {
            self.clear();
            return false;
        }
        let adapter = self.adapter.as_deref_mut().unwrap();
        adapter.stats.mode = None;
        adapter.stats.fallback = None;
        adapter.stats.animation_parity_unavailable = false;
        adapter.stats.lighting_parity_unavailable = false;
        adapter.stats.avatar_model_identity_unavailable = false;
        adapter.stats.gpu_rejections = self.gate.as_deref().unwrap().rejection_count();
        let result = self.publish(runtime, world, first_person, hidden, viewport);
        let adapter = self.adapter.as_deref_mut().unwrap();
        let gate = self.gate.as_deref().unwrap();
        match result {
            Ok(token) => {
                adapter.stats.mode = Some(ViewmodelMode::EmptyHandNeutralStaticFallback);
                adapter.stats.animation_parity_unavailable = true;
                adapter.stats.lighting_parity_unavailable = true;
                adapter.stats.avatar_model_identity_unavailable = true;
                adapter.stats.neutral_eligible_frames =
                    adapter.stats.neutral_eligible_frames.saturating_add(1);
                let completed = gate.completed(token);
                if !completed {
                    adapter.stats.cpu_fallback_requested_frames = adapter
                        .stats
                        .cpu_fallback_requested_frames
                        .saturating_add(1);
                }
                completed
            }
            Err(reason) => {
                self.scene.as_deref_mut().unwrap().clear(gate);
                adapter.stats.fallback = Some(reason);
                adapter.stats.cpu_fallback_requested_frames = adapter
                    .stats
                    .cpu_fallback_requested_frames
                    .saturating_add(1);
                false
            }
        }
    }
    fn publish(
        &mut self,
        runtime: &UiRuntime,
        world: &ClientWorld,
        first_person: bool,
        hidden: bool,
        viewport: [u32; 2],
    ) -> Result<ViewmodelToken, HandFallback> {
        if hidden
            || !first_person
            || runtime.ui_focused()
            || runtime
                .player_game_mode()
                .is_some_and(|mode| !mode.shows_hotbar())
        {
            return Err(HandFallback::Hidden);
        }
        let stream = world.stream.as_ref().ok_or(HandFallback::Ownership)?;
        let actor = stream
            .actor(stream.local_player_runtime_id())
            .ok_or(HandFallback::Ownership)?;
        let rig = stream
            .actor_rig(actor.runtime_id)
            .ok_or(HandFallback::Ownership)?;
        if rig.actor.session_id != stream.actor_session_id()
            || rig.actor.spawn_revision != actor.spawn_revision
            || rig.actor.runtime_id != actor.runtime_id
            || runtime.session_id() == 0
            || runtime.local_runtime_id() != Some(stream.local_player_runtime_id())
        {
            return Err(HandFallback::Ownership);
        }
        if stream
            .actor_health_by_unique(actor.unique_id)
            .is_some_and(|(health, _)| health <= 0.)
            || runtime
                .gameplay_hud()
                .air_ticks()
                .is_some_and(|(air, max)| air < max)
            || matches!(actor.metadata.get(&0), Some(protocol::ActorMetadataValue::Flags(flags))
                if flags & ((1 << 4) | (1 << 5)) != 0)
        {
            return Err(HandFallback::KnownActive);
        }
        // The retained scale field is a wire observation, not a fresh clock or
        // an inferred animation state. This profile supplies only ordinary scale.
        if actor.metadata.get(&38).is_some_and(|value| {
            !matches!(value,
            protocol::ActorMetadataValue::Float(scale) if *scale == 1.0)
        }) {
            return Err(HandFallback::Geometry);
        }
        if !matches!(
            runtime
                .selected_stack_snapshot()
                .map(|snapshot| snapshot.state),
            Some(crate::ui_runtime::inventory_ledger::PlayerInventorySlot::Empty)
        ) || runtime.gameplay_hud().offhand_is_empty() != Some(true)
        {
            return Err(HandFallback::ItemsUnknownOrHeld);
        }
        let (owner, camera, target, msaa, hdr) =
            self.cameras.single().map_err(|_| HandFallback::View)?;
        if !camera.is_active
            || camera.viewport.is_some()
            || !matches!(target, RenderTarget::Window(WindowRef::Primary))
            || camera.physical_viewport_size() != Some(UVec2::from_array(viewport))
        {
            return Err(HandFallback::View);
        }
        let geometry = self.geometry.as_ref().ok_or(HandFallback::Geometry)?;
        if !geometry.accepts_rig(rig.rig.0) {
            return Err(HandFallback::Geometry);
        }
        let profile = stream
            .actor_player_profile(actor.runtime_id)
            .ok_or(HandFallback::Skin)?;
        let protocol::PlayerSkin::Standard(raw) = &profile.skin else {
            return Err(HandFallback::Skin);
        };
        let adapter = self.adapter.as_deref_mut().unwrap();
        let skin = adapter.skin(raw).ok_or(HandFallback::Skin)?;
        let token = ViewmodelToken {
            session: runtime.session_id(),
            actor_session: rig.actor.session_id,
            dimension: rig.actor.dimension,
            runtime: actor.runtime_id,
            spawn: actor.spawn_revision,
            owner,
            viewport,
            samples: msaa.samples(),
            hdr: hdr.is_some(),
            skin: adapter.skin_identity,
            geometry: ViewmodelScene::geometry_identity(geometry),
            revision: adapter.revision,
        };
        if !self.scene.as_deref_mut().unwrap().publish(
            token,
            &skin,
            geometry,
            self.gate.as_deref().unwrap(),
        ) {
            return Err(HandFallback::View);
        }
        Ok(token)
    }
}
