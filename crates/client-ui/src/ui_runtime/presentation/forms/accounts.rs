//! Saved accounts use the vanilla popup, button and scrolling-panel controls.

use json_ui::{Catalog, CollectionItem, DataSource, HitRegion, Scalar};

use crate::menu::{MenuAction, MenuScreen, MenuView, auth::AuthState};

pub(super) use json_ui::ACCOUNTS_SCREEN as SCREEN;

pub(super) fn extend_catalog(catalog: &mut Catalog) {
    catalog.overlay_text("ui/cinnabar_accounts.json", include_str!("accounts.json"));
}

pub(super) fn data(view: &MenuView) -> DataSource {
    let mut data = DataSource::new();
    data.set_strict(true);
    let busy = view.feeds.account_adding;
    data.set_global("#accounts_busy", Scalar::Bool(busy));
    data.set_global("#accounts_ready", Scalar::Bool(!busy));
    data.set_global(
        "#account_count",
        Scalar::Num(view.feeds.accounts.len() as f64),
    );
    data.set_global(
        "#account_error",
        Scalar::Text(view.feeds.account_error.clone().unwrap_or_default()),
    );
    let status = match &view.auth_state {
        AuthState::AwaitingCode { uri, code } => {
            format!("Finish signing in in the browser.\n\n{uri}\n\n{code}")
        }
        AuthState::Failed(reason) => reason.clone(),
        AuthState::Authenticated => "Loading Xbox profile…".into(),
        _ => "Opening Microsoft sign-in…".into(),
    };
    data.set_global("#account_status", Scalar::Text(status));
    data.set_collection(
        "cinnabar_accounts",
        view.feeds
            .accounts
            .iter()
            .map(|account| {
                let current = view.feeds.account_active_id.as_deref() == Some(account.id.as_str());
                CollectionItem::default()
                    .with(
                        "#account_name",
                        Scalar::Text(if current {
                            format!("{} · Current", account.gamertag)
                        } else {
                            account.gamertag.clone()
                        }),
                    )
                    .with(
                        "#account_picture",
                        Scalar::Text(
                            account
                                .picture_path
                                .clone()
                                .filter(|path| !path.is_empty())
                                .unwrap_or_else(|| "textures/ui/icon_alex".into()),
                        ),
                    )
                    .with("#account_enabled", Scalar::Bool(!busy && !current))
            })
            .collect(),
    );
    data
}

pub(super) fn action(view: &MenuView, region: &HitRegion) -> Option<MenuAction> {
    Some(match region.pressed.as_deref()? {
        "button.cinnabar_switch_account" if !view.feeds.account_adding => {
            MenuAction::SwitchAccount(region.collection_index?)
        }
        "button.cinnabar_add_account" if !view.feeds.account_adding => MenuAction::AddAccount,
        "button.cinnabar_cancel_account" => MenuAction::CancelSignIn,
        "button.cinnabar_view_profile" if !view.feeds.account_adding => {
            MenuAction::Navigate(MenuScreen::Profile)
        }
        "button.cinnabar_accounts_close" if view.feeds.account_adding => MenuAction::CancelSignIn,
        "button.cinnabar_accounts_close" => MenuAction::DismissDialog,
        _ => return None,
    })
}

pub(super) fn current_name(view: &MenuView) -> &str {
    if !view.feeds.profile.gamertag.is_empty() {
        return &view.feeds.profile.gamertag;
    }
    view.feeds
        .accounts
        .iter()
        .find(|account| Some(&account.id) == view.feeds.account_active_id.as_ref())
        .map_or(view.display_name.as_str(), |account| {
            account.gamertag.as_str()
        })
}

pub(super) fn current_picture(view: &MenuView) -> Option<&str> {
    if !view.feeds.profile.picture_path.is_empty() {
        return Some(&view.feeds.profile.picture_path);
    }
    view.feeds
        .accounts
        .iter()
        .find(|account| Some(&account.id) == view.feeds.account_active_id.as_ref())
        .and_then(|account| account.picture_path.as_deref())
        .filter(|path| !path.is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::{
        pack_harness::{drawn_texts, engine_presentation, menu_nodes},
        test_support::draw_menu_actions,
    };
    use super::*;
    use crate::menu::MenuDialog;

    fn manager_view() -> MenuView {
        let mut view = MenuView::new(true, "First".into());
        view.auth_state = AuthState::Authenticated;
        view.dialog = Some(MenuDialog::Accounts);
        view.feeds.account_active_id = Some("first".into());
        view.feeds.accounts = vec![
            launcher::accounts::AccountProfile {
                id: "first".into(),
                gamertag: "First".into(),
                picture_path: Some("first.png".into()),
            },
            launcher::accounts::AccountProfile {
                id: "second".into(),
                gamertag: "Second".into(),
                picture_path: Some("second.png".into()),
            },
        ];
        view
    }

    #[test]
    fn saved_gamerpics_bind_their_file_paths_to_image_textures() {
        let mut catalog = Catalog::default();
        extend_catalog(&mut catalog);
        let context = super::super::menu_screens::retail_context();
        let root = json_ui::resolve(&catalog, "cinnabar_accounts.list", &context)
            .control
            .unwrap();
        let library = json_ui::CatalogLibrary {
            catalog: &catalog,
            context: &context,
        };
        let bound = json_ui::bind(&root, &data(&manager_view()), &library);
        fn images<'a>(control: &'a json_ui::ResolvedControl, paths: &mut Vec<&'a str>) {
            if control.control_type.as_deref() == Some("image")
                && let Some(path) = control
                    .properties
                    .get("texture")
                    .and_then(serde_json::Value::as_str)
            {
                paths.push(path);
            }
            for child in &control.children {
                images(child, paths);
            }
        }
        let mut paths = Vec::new();
        images(&bound, &mut paths);
        assert_eq!(paths, ["first.png", "second.png"]);
    }

    #[test]
    fn home_portrait_uses_the_saved_xbox_picture_until_profile_arrives() {
        let mut view = manager_view();
        assert_eq!(current_picture(&view), Some("first.png"));
        assert_eq!(current_name(&view), "First");
        view.feeds.profile.picture_path = "fresh.png".into();
        assert_eq!(current_picture(&view), Some("fresh.png"));
        view.feeds.profile.picture_path.clear();
        view.feeds.account_active_id = Some("second".into());
        assert_eq!(current_picture(&view), Some("second.png"));
        assert_eq!(current_name(&view), "Second");
        view.feeds.profile.gamertag = "Fresh name".into();
        assert_eq!(current_name(&view), "Fresh name");
    }

    #[test]
    fn account_picker_draws_cached_gamerpic_pixels() {
        let Some(mut presentation) = engine_presentation() else {
            eprintln!(
                "skipping account gamerpic rendering test: missing local UI carrier; make assets"
            );
            return;
        };
        let dir = std::env::temp_dir().join(format!("cinnabar-gamerpic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let picture = dir.join("picture.png");
        if let Some(source) = std::env::var_os("CINNABAR_GAMERPIC_FIXTURE") {
            if let Err(error) = std::fs::copy(&source, &picture) {
                if error.kind() == std::io::ErrorKind::NotFound {
                    eprintln!(
                        "skipping account gamerpic rendering test: missing fixture {}",
                        std::path::Path::new(&source).display()
                    );
                    std::fs::remove_dir_all(dir).unwrap();
                    return;
                }
                panic!("copy account gamerpic fixture: {error}");
            }
        } else {
            image::RgbaImage::from_pixel(2, 2, image::Rgba([170, 40, 80, 255]))
                .save(&picture)
                .unwrap();
        }
        let mut view = manager_view();
        let path = picture.to_string_lossy().into_owned();
        view.feeds.accounts[0].picture_path = Some(path.clone());
        presentation.sync_menu_artwork(super::super::super::menu_artwork::view_paths(&view));
        presentation.finish_menu_artwork();
        let icon = presentation
            .menu_artwork_icon(&path)
            .expect("decoded cached gamerpic");
        draw_menu_actions(&player_state::PlayerState::new(1), &mut presentation, &view);
        assert!(menu_nodes(&presentation).iter().any(|node| matches!(node.visual(), ui::UiVisual::Sprite { texture_page, uv, .. } if *texture_page == icon.page && *uv == icon.uv)), "account picture must sample its decoded artwork");
        let input = presentation.last_input.as_ref().unwrap();
        super::super::snapshot::write(input, "account-picker-pixels");
        if std::env::var_os("CINNABAR_GAMERPIC_FIXTURE").is_none() {
            let pixels = super::super::snapshot::rasterize(input);
            assert!(
                pixels
                    .pixels()
                    .filter(|pixel| pixel.0 == [170, 40, 80, 255])
                    .count()
                    > 100,
                "decoded account picture must remain visible after compositing"
            );
        }
        view.dialog = None;
        draw_menu_actions(&player_state::PlayerState::new(1), &mut presentation, &view);
        assert!(menu_nodes(&presentation).iter().any(|node| matches!(node.visual(), ui::UiVisual::Sprite { texture_page, uv, .. } if *texture_page == icon.page && *uv == icon.uv)), "Home profile must sample the cached Xbox picture");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn account_popup_only_exposes_saved_switch_and_manager_actions() {
        let Some(mut presentation) = engine_presentation() else {
            eprintln!("skipping account popup action test: missing local UI carrier; make assets");
            return;
        };
        let actions = draw_menu_actions(
            &player_state::PlayerState::new(1),
            &mut presentation,
            &manager_view(),
        );
        let texts = drawn_texts(menu_nodes(&presentation));
        assert!(texts.iter().any(|text| text.contains("First")), "{texts:?}");
        assert!(
            texts.iter().any(|text| text.contains("Second")),
            "{texts:?}"
        );
        assert!(
            actions.contains(&MenuAction::SwitchAccount(1)),
            "{actions:?}"
        );
        assert!(
            !actions.contains(&MenuAction::SwitchAccount(0)),
            "current account cannot switch to itself"
        );
        for action in [
            MenuAction::AddAccount,
            MenuAction::Navigate(MenuScreen::Profile),
            MenuAction::DismissDialog,
        ] {
            assert!(actions.contains(&action), "missing {action:?}: {actions:?}");
        }
        assert!(
            !actions.contains(&MenuAction::Navigate(MenuScreen::Play)),
            "popup blocks the main menu"
        );
    }

    #[test]
    fn pending_sign_in_blocks_switching_and_closing_cancels_it() {
        let Some(mut presentation) = engine_presentation() else {
            eprintln!("skipping pending account popup test: missing local UI carrier; make assets");
            return;
        };
        let mut view = manager_view();
        view.feeds.account_adding = true;
        view.auth_state = AuthState::AwaitingCode {
            uri: "https://example.invalid/link".into(),
            code: "TEST".into(),
        };
        let actions =
            draw_menu_actions(&player_state::PlayerState::new(1), &mut presentation, &view);
        let texts = drawn_texts(menu_nodes(&presentation));
        assert!(texts.iter().any(|text| text.contains("TEST")), "{texts:?}");
        assert!(actions.contains(&MenuAction::CancelSignIn), "{actions:?}");
        assert!(!actions.iter().any(|action| matches!(
            action,
            MenuAction::SwitchAccount(_) | MenuAction::AddAccount | MenuAction::DismissDialog
        )));
    }
}
