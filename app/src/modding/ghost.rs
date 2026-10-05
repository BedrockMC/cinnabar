//! Publishes the own last-forwarded entity bounds without observing other players.

use super::packet_delay::RealPositionSnapshot;
use crate::{
    app::ClientFrameSet, camera::FlyCamera, menu::MenuRuntime, runtime::world::ClientWorld,
};
use bevy::{prelude::*, window::PrimaryWindow};
use client_ui::ui_runtime::{
    UiRuntime,
    presentation::{UiPresentationRuntime, forms::mod_ghost::project_entity_rectangle},
};

pub(super) fn configure(app: &mut App) {
    app.add_systems(
        Update,
        publish
            .after(super::packet_delay::publish_packet_delay)
            .after(ClientFrameSet::Camera)
            .before(ClientFrameSet::UiPreparation),
    );
}

pub(crate) fn publish(
    snapshot: Option<Res<RealPositionSnapshot>>,
    world: Option<Res<ClientWorld>>,
    menu: Option<Res<MenuRuntime>>,
    ui: Option<Res<UiRuntime>>,
    player: Option<Res<crate::player_runtime::PlayerRuntime>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Projection, &Transform), With<FlyCamera>>,
    mut presentation: ResMut<UiPresentationRuntime>,
) {
    let bounds = if world
        .as_deref()
        .is_none_or(|world| world.stream.is_none() || world.fatal_error.is_some())
        || menu.as_deref().is_some_and(MenuRuntime::is_visible)
        || ui
            .as_deref()
            .zip(player.as_deref())
            .is_none_or(|(ui, player)| ui.ui_focused(player))
    {
        None
    } else {
        snapshot
            .as_deref()
            .filter(|snapshot| snapshot.session_id != 0)
            .and_then(|snapshot| snapshot.position)
            .and_then(|feet| {
                let (window, (projection, transform)) =
                    (windows.single().ok()?, cameras.single().ok()?);
                let aabb = sim::Aabb::player_at(sim::Vec3::new(
                    feet[0] as f64,
                    feet[1] as f64,
                    feet[2] as f64,
                ));
                let point = |v: sim::Vec3| [v.x as f32, v.y as f32, v.z as f32];
                project_entity_rectangle(
                    transform.translation,
                    projection.get_clip_from_view() * transform.to_matrix().inverse(),
                    [point(aabb.min), point(aabb.max)],
                    [window.physical_width(), window.physical_height()],
                )
            })
    };
    if let Err(error) = presentation.set_mod_position_rectangle(bounds) {
        bevy::log::warn!(%error, "own-position marker disabled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_of_current_world_clears_a_previously_painted_rectangle() {
        let mut presentation = client_ui::test_support::mini_engine_presentation();
        presentation.set_server_ui_pack(&client_ui::ui_runtime::presentation::ServerUiPack {
            ui_layers: vec![vec![
                (
                    "ui/_ui_defs.json".into(),
                    br#"{"ui_defs":["ui/hud_screen.json"]}"#.to_vec(),
                ),
                (
                    "ui/hud_screen.json".into(),
                    br#"{"namespace":"hud","hud_screen":{"type":"screen","absorbs_input":false,"is_showing_menu":false,"size":["100%","100%"]}}"#
                        .to_vec(),
                ),
            ]],
            ..Default::default()
        });
        let player = crate::player_runtime::PlayerRuntime::new(1);
        let ui = UiRuntime::new(1);
        let draw = |presentation: &mut UiPresentationRuntime| {
            presentation
                .build(&player, &ui, 0, [800, 600], ui::DpiScale::new(1.0).unwrap())
                .unwrap()
        };
        let baseline = draw(&mut presentation);
        presentation
            .set_mod_position_rectangle(Some([250, 120, 390, 480]))
            .unwrap();
        assert_ne!(draw(&mut presentation).vertices, baseline.vertices);
        let mut app = App::new();
        app.insert_resource(presentation)
            .insert_resource(RealPositionSnapshot {
                position: Some([0.0, 0.0, -3.0]),
                session_id: 1,
            })
            .add_systems(Update, publish);
        app.update();
        assert_eq!(
            draw(&mut app.world_mut().resource_mut::<UiPresentationRuntime>()).vertices,
            baseline.vertices
        );
    }
}
