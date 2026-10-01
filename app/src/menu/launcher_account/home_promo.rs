//! Offline home-feed mapping and rendered start-screen evidence.

use super::*;

#[test]
fn home_feed_preserves_promo_and_fallback_label() {
    let mut home = Home::default();
    home.live_events
        .push(protocol::launcher_control::LiveEvent {
            caption_text: "Live now".into(),
            route_to_servers: true,
            ..Default::default()
        });
    let card = menu_home(&home, 0).live_event.unwrap();
    assert_eq!(card.button_text, "gathering.button.liveEventFallback");
    assert_eq!(card.caption, "Live now");
    assert!(card.route_to_servers);
    home.live_events[0].button_text = "Learn More".into();
    assert_eq!(
        menu_home(&home, 0).live_event.unwrap().button_text,
        "Learn More"
    );
}

#[test]
fn snapshot_core_home_promo() {
    let Ok(path) = std::env::var("CINNABAR_HOME_PROMO_FIXTURE") else {
        return;
    };
    let mut home: Home = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let badge = std::path::Path::new(&path).with_file_name("offline-promo-badge.png");
    image::RgbaImage::from_pixel(256, 128, image::Rgba([40, 90, 200, 255]))
        .save(&badge)
        .unwrap();
    home.live_events[0].badge.path = badge.to_string_lossy().into_owned();
    let mut view = crate::menu::MenuRuntime::new(true, 2, "Steve".into()).view();
    view.auth_state = AuthState::Authenticated;
    view.catalog_loading = false;
    view.feeds.home = menu_home(&home, 0);
    let mut presentation =
        crate::ui_runtime::presentation::forms::pack_harness::engine_presentation()
            .expect("the offline promo snapshot requires the real UI carrier");
    presentation.sync_menu_artwork(vec![(badge.to_string_lossy().into_owned(), 256)]);
    presentation.finish_menu_artwork();
    let runtime = crate::ui_runtime::UiRuntime::new(1);
    let dpi = ui::DpiScale::new(2.0).unwrap();
    for _ in 0..3 {
        presentation.set_menu_view(Some(view.clone()));
        presentation.build(&runtime, 0, [2560, 1440], dpi).unwrap();
    }
    presentation.set_menu_view(Some(view));
    let input = presentation.build(&runtime, 0, [2560, 1440], dpi).unwrap();
    let frame = crate::ui_runtime::presentation::forms::snapshot::rasterize(&input);
    assert!(
        frame.pixels().any(|pixel| pixel.0 == [40, 90, 200, 255]),
        "promo badge was not rendered"
    );
    crate::ui_runtime::presentation::forms::snapshot::write(&input, "home-promo");
}
