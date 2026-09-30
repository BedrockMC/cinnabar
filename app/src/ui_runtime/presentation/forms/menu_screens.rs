//! Which vanilla screen each menu state opens, the screen globals it binds from
//! the menu's view, and how the screen's pressed buttons map back to menu
//! actions. Screens bind strictly (an unbound visibility flag reads false, as
//! the vanilla screen controllers answer it), so each spec names the flags its
//! layout needs on.

use std::sync::Arc;

use json_ui::{Context, DataSource, HitKind, HitRegion, Scalar};
use serde_json::Value;

use super::play_screen;
use crate::menu::{
    MenuAction, MenuDialog, MenuScreen, MenuView, VOLUME_SLIDERS, VOLUME_STEPS, auth::AuthState,
};

const SETTINGS_SECTIONS: &[(&str, u8)] = &[
    ("server_forced_index", 1),
    ("accessibility_forced_index", 2),
    ("how_to_play_index", 3),
    ("game_forced_index", 4),
    ("classroom_forced_index", 5),
    ("edu_cloud_level_forced_index", 6),
    ("multiplayer_forced_index", 7),
    ("world_forced_index", 8),
    ("members_forced_index", 9),
    ("realms_saves_forced_index", 10),
    ("subscription_forced_index", 11),
    ("backup_forced_index", 12),
    ("dev_options_forced_index", 13),
    ("keyboard_and_mouse_forced_index", 14),
    ("controller_and_switch_forced_index", 15),
    ("touch_forced_index", 16),
    ("party_forced_index", 17),
    ("general_forced_index", 18),
    ("account_forced_index", 19),
    ("creator_forced_index", 20),
    ("video_forced_index", 21),
    ("view_subscriptions_forced_index", 22),
    ("sound_forced_index", 23),
    ("global_texture_pack_forced_index", 24),
    ("storage_management_forced_index", 25),
    ("edu_cloud_storage_forced_index", 26),
    ("language_forced_index", 27),
    ("preview_forced_index", 28),
    ("debug_forced_index", 29),
    ("discovery_debug_forced_index", 30),
    ("ui_debug_forced_index", 31),
    ("edu_debug_forced_index", 32),
    ("marketplace_debug_forced_index", 33),
    ("flighting_debug_forced_index", 34),
    ("realms_debug_forced_index", 35),
    ("automation_forced_index", 36),
    ("level_texture_pack_index", 37),
    ("broadcast_forced_index", 38),
    ("addon_index", 39),
    ("invite_links_forced_index", 40),
    ("general_invite_link_forced_index", 41),
    ("advanced_invite_link_forced_index", 42),
    ("realms_advanced_forced_index", 43),
];
/// The section the settings screen opens on before one is picked.
const VIDEO_SECTION: &str = "video_forced_index";
/// GUI scale choices the settings slider steps through (1..=4).
const GUI_SCALE_STEPS: f64 = 4.0;

/// The modal progress screen joining a server shows (bare `progress_screen` has no content).
const JOIN_PROGRESS_SCREEN: &str = "progress.world_convert_modal_progress_screen";

/// Lang key the vanilla start and pause controllers give the unlock-full-game text.
const UNLOCK_FULL_GAME_TEXT: &str = "trial.pauseScreen.buyGame";

/// The retail desktop context for this build's platform.
pub(super) fn retail_context() -> Context {
    Context::retail(cfg!(target_os = "macos"))
}

/// `StartMenuScreenController::addStaticScreenVars` for a full-game, non-edu
/// account: demo, edu and unlock controls stay ignored.
fn start_screen_vars(context: Context) -> Context {
    unlock_text(context)
        .with_flag("unlock_full_game_button_ignored", true)
        .with_flag("featured_world_ignored", true)
        .with_flag("courses_ignored", true)
        .with_flag("edu_feedback_ignored", true)
        .with_flag("play_button_visible", true)
        .with_flag("use_single_column_for_buttons", false)
        .with_flag("can_swap_vr_mode", false)
        .with_flag("showing_new_player_flow_buttons", false)
        .with_flag("supports_launching_legacy_version", false)
}

fn unlock_text(context: Context) -> Context {
    context.with_var(
        "unlock_full_game_button_text",
        Value::String(UNLOCK_FULL_GAME_TEXT.into()),
    )
}

/// The screen a menu state opens and what it binds.
pub(super) struct MenuScreenData {
    pub(super) reference: &'static str,
    pub(super) context: Context,
    pub(super) data: DataSource,
    /// A screen drawn over this one, which then takes all input (a Marketplace popup).
    pub(super) overlay: Option<Box<MenuScreenData>>,
}

type Translate<'a> = &'a dyn Fn(&str) -> Option<Arc<str>>;

fn text(value: impl Into<String>) -> Scalar {
    Scalar::Text(value.into())
}

fn translated(translate: Translate<'_>, key: &str, fallback: &str) -> String {
    translate(key).map_or_else(|| fallback.to_owned(), |value| value.to_string())
}

fn flags(data: &mut DataSource, on: &[&str]) {
    for name in on {
        data.set_global(*name, Scalar::Bool(true));
    }
}

/// The vanilla screen for `view`, or `None` for states without one (the
/// programmatic launcher then draws them).
pub(super) fn screen_data(view: &MenuView, translate: Translate<'_>) -> Option<MenuScreenData> {
    let mut data = DataSource::new();
    data.set_strict(true);
    let mut context = base_context();
    let reference = if let Some((received, total)) =
        view.feeds.pack_download.filter(|_| view.connecting)
    {
        pack_download(&mut data, translate, received, total);
        JOIN_PROGRESS_SCREEN
    } else if view.connecting {
        data.set_global(
            "#title_text",
            text(translated(translate, "connect.connecting", "Connecting")),
        );
        data.set_global(
            "#progress_text",
            text(view.message.clone().unwrap_or_default()),
        );
        flags(&mut data, &["#bar_animation_visible"]);
        JOIN_PROGRESS_SCREEN
    } else if let Some(error) = &view.disconnect_message {
        let words = crate::menu::disconnect::describe(error);
        data.set_global(
            "#title_text",
            text(translated(translate, words.title, words.title)),
        );
        let body = match words.body {
            crate::menu::disconnect::DisconnectBody::Key(key) => translated(translate, key, key),
            crate::menu::disconnect::DisconnectBody::Server(message) => message,
        };
        data.set_global("#disconnect_text", text(body));
        "disconnect.disconnect_screen"
    } else if let AuthState::AwaitingCode { uri, code } = &view.auth_state {
        data.set_global("#url", text(uri.clone()));
        data.set_global("#code", text(code.clone()));
        "xbl_console_signin.xbl_console_signin"
    } else {
        match view.screen {
            MenuScreen::Death => {
                flags(
                    &mut data,
                    &[
                        "#buttons_and_deathmessage_visible",
                        "#respawn_visible",
                        "#respawn_enabled",
                        "#quit_visible",
                        "#quit_enabled",
                    ],
                );
                "death.death_screen"
            }
            MenuScreen::Pause => {
                data.set_global("#playername", text(view.display_name.clone()));
                flags(&mut data, &["#playername_visible"]);
                data.set_global("#unlock_full_game_button_text", text(UNLOCK_FULL_GAME_TEXT));
                // A non-edu client draws the retail pause content, not edu_pause's.
                context = unlock_text(context)
                    .with_flag("ignore_edu_pause", true)
                    .with_var(
                        "store_button_text",
                        Value::String(server_store_text(translate)),
                    );
                "pause.pause_screen"
            }
            MenuScreen::Home => {
                start_screen(view, &mut data, translate);
                context = start_screen_vars(context);
                "start.start_screen"
            }
            MenuScreen::Play | MenuScreen::Social | MenuScreen::Servers => {
                super::play_screen::bind(view, &mut data);
                "play.play_screen"
            }
            MenuScreen::AddServer => {
                add_server_screen(view, &mut data, translate);
                // The controller's edit mode swaps Play for Remove.
                context = context.with_flag("edit_mode", view.editing.is_some());
                "add_external_server.add_external_server_screen_new"
            }
            MenuScreen::Settings => {
                settings_screen(view, &mut data);
                super::settings_defaults::bind(&mut data, &|key: &str| {
                    translated(translate, key, key)
                });
                return Some(MenuScreenData {
                    reference: SETTINGS_SCREEN,
                    context: settings_context(context),
                    data,
                    overlay: None,
                });
            }
            MenuScreen::Store => return store_screen(view, &context, translate),
            MenuScreen::Profile | MenuScreen::Inbox | MenuScreen::Friends => return None,
        }
    };
    Some(MenuScreenData {
        reference,
        context,
        data,
        overlay: None,
    })
}

/// The progress screen while the core downloads the server's packs: the
/// "Downloading packs" title with the percent and a determinate bar.
fn pack_download(data: &mut DataSource, translate: Translate<'_>, received: u64, total: u64) {
    let percent = if total == 0 {
        0
    } else {
        (received.min(total) * 100 / total) as u32
    };
    let title = translated(
        translate,
        "progressScreen.title.downloading",
        "Downloading packs %1",
    );
    data.set_global(
        "#title_text",
        text(title.replace("%1", &format!("{percent}%"))),
    );
    let megabytes = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
    data.set_global(
        "#progress_text",
        text(format!(
            "{:.1} / {:.1} MB",
            megabytes(received),
            megabytes(total)
        )),
    );
    flags(data, &["#loading_bar_visible"]);
    data.set_global(
        "#loading_bar_total_amount",
        Scalar::Num(total.max(1) as f64),
    );
    data.set_global(
        "#loading_bar_current_amount",
        Scalar::Num(received.min(total) as f64),
    );
}

/// The Marketplace screen (and popup) for the published store state.
fn store_screen(
    view: &MenuView,
    context: &Context,
    translate: Translate<'_>,
) -> Option<MenuScreenData> {
    let snapshot = view.store.as_deref()?;
    let tr = |key: &str| translated(translate, key, key);
    let screens = crate::store::screens(snapshot, context, &tr);
    let convert = |spec: crate::store::ScreenSpec| MenuScreenData {
        reference: spec.reference,
        context: spec.context,
        data: spec.data,
        overlay: None,
    };
    let mut base = convert(screens.base);
    base.overlay = screens.overlay.map(|spec| Box::new(convert(spec)));
    Some(base)
}

fn start_screen(view: &MenuView, data: &mut DataSource, translate: Translate<'_>) {
    let profile = &view.feeds.profile;
    let gamertag = if profile.gamertag.is_empty() {
        view.display_name.clone()
    } else {
        profile.gamertag.clone()
    };
    data.set_global("#playername", text(gamertag.clone()));
    data.set_global("#gamertag_label", text(gamertag));
    let portrait = !profile.picture_path.is_empty() || !view.feeds.home.persona_head.is_empty();
    data.set_global("#show_gamerpic", Scalar::Bool(portrait));
    flags(data, &["#show_paper_doll", "#persona_and_skins_enabled"]);
    super::start_feed::bind(view, data);
    data.set_global("#version", text(version_label(protocol::GAME_VERSION)));
    data.set_global("#unlock_full_game_button_text", text(UNLOCK_FULL_GAME_TEXT));
    data.set_global("#edu_demo_only_ui_visible", Scalar::Bool(false));
    // Retail Realms is enabled, so its row shows between Settings and
    // Marketplace, as on the release client.
    flags(
        data,
        &[
            "#online_stack_visible",
            "#realms_promo_visible",
            "#not_realms_promo_visible_and_supports_launching_legacy_version",
            "#dressing_room_button_visible",
            "#is_appearance_visible",
        ],
    );
    match &view.auth_state {
        AuthState::SignedOut | AuthState::Failed(_) => {
            flags(data, &["#sign_in_visible", "#upper_online_buttons_visible"])
        }
        AuthState::Checking => {
            flags(data, &["#signingin_visible"]);
            data.set_global(
                "#signingin_text",
                text(translated(
                    translate,
                    "xbox.signingin",
                    "Signing in with your Microsoft account...",
                )),
            );
        }
        AuthState::Authenticated => flags(data, &["#gamertag_pic_and_label_visible"]),
        AuthState::AwaitingCode { .. } => {}
    }
}

/// The pause store button on a third-party server, as `PauseScreenController`
/// names it: "%s Store" with the server's store name, else the generic "Server".
fn server_store_text(translate: Translate<'_>) -> String {
    let server = translated(translate, "menu.serverGenericName", "Server");
    translated(translate, "menu.serverStore", "%s Store").replacen("%s", &server, 1)
}

/// The start screen's version: the release client shows `1.26.50` as `v26.50`.
fn version_label(game_version: &str) -> String {
    format!(
        "v{}",
        game_version.strip_prefix("1.").unwrap_or(game_version)
    )
}

/// The vanilla two-button popup a launcher dialog opens, and the action its
/// left (confirm) button takes; the right button dismisses.
pub(super) fn dialog_model(
    view: &MenuView,
    dialog: MenuDialog,
    translate: Translate<'_>,
) -> (json_ui::FormModel, MenuAction) {
    let (title, body, button1, button2, confirm) = match dialog {
        MenuDialog::Exit => (
            translated(
                translate,
                "gui.warning.exitGameWarning",
                "Do you want to exit Minecraft?",
            ),
            String::new(),
            translated(translate, "gui.yes", "Yes"),
            translated(translate, "gui.no", "No"),
            MenuAction::ConfirmExit,
        ),
        // The popup's title is one line, so the server names it and the body asks.
        MenuDialog::RemoveSaved(index) => (
            view.servers
                .get(index)
                .map(|server| server.name.clone())
                .unwrap_or_default(),
            translated(
                translate,
                "addExternalServerScreen.removeConfirmation",
                "Are you sure you want to remove this server?",
            ),
            translated(
                translate,
                "addExternalServerScreen.removeButtonLabel",
                "Remove",
            ),
            translated(translate, "gui.cancel", "Cancel"),
            MenuAction::ConfirmRemoveSaved(index),
        ),
    };
    let model = json_ui::FormModel::Modal(json_ui::ModalForm {
        title,
        body,
        button1,
        button2,
    });
    (model, confirm)
}

fn add_server_screen(view: &MenuView, data: &mut DataSource, translate: Translate<'_>) {
    let (ip, port) = split_address(&view.address);
    let title = if view.editing.is_some() {
        translated(translate, "addServer.title.edit", "Edit Server")
    } else {
        translated(translate, "addServer.title", "Add Server")
    };
    data.set_global("#title_text", text(title));
    data.set_global("#name_text_box_content", text(view.name.clone()));
    data.set_global("#ip_text_box_content", text(ip));
    data.set_global("#port_text_box_content", text(port));
    let ready = !view.name.trim().is_empty() && !view.address.trim().is_empty();
    data.set_global("#save_button_enabled", Scalar::Bool(ready));
    data.set_global("#save_button_disabled", Scalar::Bool(!ready));
    data.set_global("#play_button_enabled", Scalar::Bool(ready));
    data.set_global("#play_button_disabled", Scalar::Bool(!ready));
}

/// `host:port` split for the separate IP and port boxes (a bare host keeps the
/// default Bedrock port shown).
fn split_address(address: &str) -> (String, String) {
    match address.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            (host.trim_matches(['[', ']']).to_owned(), port.to_owned())
        }
        _ => (address.to_owned(), "19132".to_owned()),
    }
}

fn settings_screen(view: &MenuView, data: &mut DataSource) {
    let section = match view.settings_section {
        0 => section_index(VIDEO_SECTION),
        picked => picked,
    };
    data.select_radio("navigation_tab", usize::from(section));
    let scale = f64::from(view.gui_scale.clamp(1, 4));
    data.set_global("#gui_scale", Scalar::Num(scale - 1.0));
    data.set_global("#gui_scale_steps", Scalar::Num(GUI_SCALE_STEPS));
    data.set_global(
        "#gui_scale_slider_label",
        text(format!("GUI Scale: {scale}")),
    );
    flags(data, &["#gui_scale_visible", "#gui_scale_enabled"]);
    for ((slider, _), percent) in VOLUME_SLIDERS.iter().zip(view.volumes) {
        let shown = percent.unwrap_or(100);
        data.set_global(format!("#{slider}"), Scalar::Num(f64::from(shown) / 100.0));
        data.set_global(format!("#{slider}_slider_label"), text(format!("{shown}%")));
        data.set_global(
            format!("#{slider}_enabled"),
            Scalar::Bool(percent.is_some()),
        );
    }
}

const SETTINGS_SCREEN: &str = "settings.screen_controls_and_settings";

/// Every launcher screen's context before its own vars.
fn base_context() -> Context {
    retail_context().with_flag("can_quit", true).with_var(
        "play_button_target",
        Value::String("button.menu_play".into()),
    )
}

/// The settings screen and the context it opens with, to resolve ahead of time.
pub(super) fn settings_prewarm() -> (&'static str, Context) {
    (SETTINGS_SCREEN, settings_context(base_context()))
}

/// The static vars `SettingsScreenController` sets for the global settings a
/// desktop client opens from the start screen: no world, realm or creation state.
fn settings_context(context: Context) -> Context {
    let flags: &[(&str, bool)] = &[
        ("include_controls_and_settings_sections", true),
        // Set when "/settings" resolves to JSON UI, as retail does with the
        // `mc-new-settings-screen` flight off.
        ("include_migrated_json_ui_settings_tabs", true),
        ("show_fullscreen_toggle", true),
        ("supports_user_configured_safezone", true),
        ("feedback_visible", true),
        ("is_global_texture_packs_visible", true),
        ("supports_cross_platform_play_toggle", false),
        ("is_world_create", false),
        ("is_world_edit", false),
        ("is_template_create", false),
        ("is_realms_edit", false),
        ("is_realm_slot", false),
        ("is_mp_host", false),
        ("is_mp_client", false),
        ("non_config_realms_env", false),
        ("realms_pack_feature_enabled", false),
        ("gamepad_supported", true),
        ("keyboard_and_mouse_supported", true),
        ("touch_supported", false),
        ("supports_flite_tts", false),
        ("platform_tts_exists", false),
        ("ignore_creator_section", false),
        ("may_include_world_section", false),
        ("ignore_global_resources_section", false),
        ("ignore_storage_section", false),
        ("ignore_profile_switch_account_button", false),
        ("ignore_profile_sso_toggle", true),
        ("ignore_profile_sign_out_button", false),
        ("ignore_controller_layout", false),
        ("edu_ignore_cloud_storage", true),
        ("storage_location_switch_enabled", false),
        ("copy_interal_storage_button_enabled", false),
        ("show_preview_button", false),
        ("show_preview_app1_button", false),
        ("show_preview_app2_button", false),
        ("debug_settings", false),
        ("party_settings_enabled", false),
        ("settings_spatial_pattern_fix_enabled", true),
        ("display_copyright_info", false),
        ("is_pregame", true),
        ("is_editor_mode_enabled", false),
    ];
    let context = flags.iter().fold(context, |context, (name, value)| {
        context.with_flag(name, *value)
    });
    SETTINGS_SECTIONS
        .iter()
        .fold(context, |context, (name, index)| {
            context.with_var(name, Value::from(*index))
        })
}

fn section_index(name: &str) -> u8 {
    SETTINGS_SECTIONS
        .iter()
        .find_map(|(section, index)| (*section == name).then_some(*index))
        .unwrap_or_default()
}

/// The menu action a pressed region means on `view`'s screen.
pub(super) fn action_for(view: &MenuView, region: &HitRegion) -> Option<MenuAction> {
    if view.screen == MenuScreen::Store {
        return crate::store::action(view.store.as_deref(), region).map(MenuAction::Store);
    }
    let index = region.collection_index;
    let collection = region.collection.as_deref();
    if region.kind == HitKind::Toggle {
        return toggle_action(view, region);
    }
    if region.kind == HitKind::EditBox {
        return match region.control_name.as_deref() {
            Some("#name_text_box") => Some(MenuAction::AddName),
            Some("#ip_text_box" | "#port_text_box") => Some(MenuAction::AddAddress),
            _ => None,
        };
    }
    Some(match region.pressed.as_deref()? {
        "button.menu_continue" if view.screen == MenuScreen::Pause => MenuAction::PauseResume,
        // Acknowledging a disconnect clears it (every action does).
        "button.menu_continue" | "button.menu_leave_screen" | "button.menu_select" => {
            MenuAction::DismissDialog
        }
        "button.menu_settings" if view.screen == MenuScreen::Pause => MenuAction::PauseSettings,
        "button.menu_settings" => MenuAction::Navigate(MenuScreen::Settings),
        "button.menu_quit" | "button.main_menu_button" => MenuAction::PauseDisconnect,
        "button.respawn_button" => MenuAction::Respawn,
        "button.gathering" => MenuAction::OpenLiveEvent,
        "button.menu_inbox" => MenuAction::Navigate(MenuScreen::Inbox),
        "button.friends_drawer" | "button.menu_friends" => {
            MenuAction::Navigate(MenuScreen::Friends)
        }
        "button.menu_store" => MenuAction::Store(crate::store::OPEN),
        "button.menu_play" => MenuAction::Navigate(MenuScreen::Play),
        "button.menu_realms" => MenuAction::Navigate(MenuScreen::Social),
        "button.menu_servers" => MenuAction::Navigate(MenuScreen::Servers),
        "button.signin" => MenuAction::StartSignIn,
        "button.sign_out" => MenuAction::SignOut,
        "button.menu_profile" | "button.to_profile_screen" | "button.manage_account" => {
            MenuAction::Navigate(MenuScreen::Profile)
        }
        "button.menu_cancel" if view.auth_state_awaiting_code() => MenuAction::CancelSignIn,
        "button.menu_exit" if view.auth_state_awaiting_code() => MenuAction::CancelSignIn,
        "button.menu_exit" => match view.screen {
            MenuScreen::Home => MenuAction::OpenExitDialog,
            _ => MenuAction::AddBack,
        },
        "button.save_button" => MenuAction::AddSave,
        "button.play_button" => MenuAction::AddSaveConnect,
        "button.remove_button" => MenuAction::RemoveSavedDialog(view.editing?),
        "button.menu_network_world_item" => match collection? {
            "friends_network_worlds" => MenuAction::PlayFriend(index?),
            "servers_network_worlds" => MenuAction::PlaySaved(index?),
            _ => return play_screen::featured_action(view, region),
        },
        "button.menu_network_server_item" | "button.connect_to_third_party_server" => {
            match collection {
                Some("servers_network_worlds") => MenuAction::PlaySaved(index?),
                _ => return play_screen::featured_action(view, region),
            }
        }
        "button.menu_network_server_world_edit" => MenuAction::EditSaved(index?),
        "button.description_read_toggle" => MenuAction::ToggleReadMore(0),
        "button.news_read_toggle" => MenuAction::ToggleReadMore(1),
        "button.menu_start_realms_world" => return play_screen::realm_action(view, region),
        "button.menu_start_local_world" => MenuAction::PlayLocalWorld(index?),
        _ => return None,
    })
}

fn toggle_action(view: &MenuView, region: &HitRegion) -> Option<MenuAction> {
    match region.control_name.as_deref()? {
        "navigation_tab" if view.screen == MenuScreen::Settings => Some(
            MenuAction::SettingsSection(u8::try_from(region.group_index?).ok()?),
        ),
        "navigation_tab" => Some(MenuAction::Navigate(match region.group_index? {
            0 => MenuScreen::Play,
            1 => MenuScreen::Social,
            _ => MenuScreen::Servers,
        })),
        "server_navigation_toggle" if region.key.contains("add_server") => {
            Some(MenuAction::PlayAddServer)
        }
        "server_navigation_toggle" if play_screen::is_featured(region) => {
            Some(MenuAction::SelectFeatured(region.collection_index?))
        }
        "server_navigation_toggle" => match region.collection.as_deref()? {
            "servers_network_worlds" => Some(MenuAction::PlaySaved(region.collection_index?)),
            _ => None,
        },
        _ => None,
    }
}

/// A settings slider's action per pointer segment, left to right: the slider
/// is split into one hit rect per value it snaps to.
pub(super) fn slider_actions(region: &HitRegion) -> Option<Vec<MenuAction>> {
    if region.kind != HitKind::Slider {
        return None;
    }
    let name = region.control_name.as_deref()?;
    if name == "gui_scale" {
        return Some(
            (1..=GUI_SCALE_STEPS as u8)
                .map(MenuAction::SettingsScale)
                .collect(),
        );
    }
    let slot = VOLUME_SLIDERS
        .iter()
        .position(|(slider, _)| *slider == name)?;
    let last = u16::from(VOLUME_STEPS - 1);
    Some(
        (0..=last)
            .map(|step| MenuAction::SettingsVolume(slot as u8, (step * 100 / last) as u8))
            .collect(),
    )
}

impl MenuView {
    fn auth_state_awaiting_code(&self) -> bool {
        matches!(self.auth_state, AuthState::AwaitingCode { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::MenuRuntime;
    use json_ui::RectOut;

    fn view(screen: MenuScreen) -> MenuView {
        let mut view = MenuRuntime::new(true, 2, "Steve".to_owned()).view();
        view.screen = screen;
        view.auth_state = AuthState::SignedOut;
        view
    }

    fn region(kind: HitKind, pressed: Option<&str>) -> HitRegion {
        let rect = RectOut {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        HitRegion {
            key: "/screen/button".to_owned(),
            name: "button".to_owned(),
            kind,
            rect,
            clip: rect,
            layer: 0,
            order: 0,
            pressed: pressed.map(str::to_owned),
            control_name: None,
            collection_index: None,
            collection: None,
            enabled: true,
            checked: None,
            max_length: None,
            group_index: None,
            renderer: None,
        }
    }

    fn reference(view: &MenuView) -> Option<&'static str> {
        screen_data(view, &|_| None).map(|screen| screen.reference)
    }

    #[test]
    fn the_pause_store_button_names_the_server_store() {
        assert_eq!(server_store_text(&|_| None), "Server Store");
    }

    #[test]
    fn the_version_reads_as_the_release_client_shows_it() {
        assert_eq!(version_label("1.26.50"), "v26.50");
        assert_eq!(version_label("26.60"), "v26.60");
    }

    #[test]
    fn menu_states_open_their_vanilla_screens() {
        assert_eq!(
            reference(&view(MenuScreen::Pause)),
            Some("pause.pause_screen")
        );
        assert_eq!(
            reference(&view(MenuScreen::Home)),
            Some("start.start_screen")
        );
        assert_eq!(
            reference(&view(MenuScreen::Death)),
            Some("death.death_screen")
        );
        assert_eq!(
            reference(&view(MenuScreen::Servers)),
            Some("play.play_screen")
        );
        let mut connecting = view(MenuScreen::Play);
        connecting.connecting = true;
        assert_eq!(reference(&connecting), Some(JOIN_PROGRESS_SCREEN));
        let mut dropped = view(MenuScreen::Play);
        dropped.disconnect_message = Some("Kicked".into());
        assert_eq!(reference(&dropped), Some("disconnect.disconnect_screen"));
        let mut code = view(MenuScreen::Home);
        code.auth_state = AuthState::AwaitingCode {
            uri: "https://x".into(),
            code: "ABC".into(),
        };
        assert_eq!(
            reference(&code),
            Some("xbl_console_signin.xbl_console_signin")
        );
    }

    #[test]
    fn pressed_buttons_map_to_menu_actions() {
        let pause = view(MenuScreen::Pause);
        let press =
            |view: &MenuView, id: &str| action_for(view, &region(HitKind::Button, Some(id)));
        assert_eq!(
            press(&pause, "button.menu_continue"),
            Some(MenuAction::PauseResume)
        );
        assert_eq!(
            press(&pause, "button.menu_quit"),
            Some(MenuAction::PauseDisconnect)
        );
        let death = view(MenuScreen::Death);
        assert_eq!(
            press(&death, "button.respawn_button"),
            Some(MenuAction::Respawn)
        );
        let home = view(MenuScreen::Home);
        assert_eq!(
            press(&home, "button.menu_exit"),
            Some(MenuAction::OpenExitDialog)
        );
        let mut edit = region(
            HitKind::Button,
            Some("button.menu_network_server_world_edit"),
        );
        edit.collection_index = Some(3);
        assert_eq!(
            action_for(&view(MenuScreen::Servers), &edit),
            Some(MenuAction::EditSaved(3))
        );
    }

    #[test]
    fn the_start_screen_marketplace_button_opens_the_store_and_its_presses_route_to_it() {
        let home = view(MenuScreen::Home);
        assert_eq!(
            action_for(&home, &region(HitKind::Button, Some("button.menu_store"))),
            Some(MenuAction::Store(crate::store::StoreAction::Open))
        );
        let mut store = view(MenuScreen::Store);
        assert!(
            reference(&store).is_none(),
            "no engine screen until the store publishes"
        );
        store.store = Some(std::sync::Arc::new(crate::store::StoreSnapshot::empty()));
        assert_eq!(
            reference(&store),
            Some("store_layout.store_data_driven_screen")
        );
        assert_eq!(
            action_for(&store, &region(HitKind::Button, Some("button.menu_exit"))),
            Some(MenuAction::Store(crate::store::StoreAction::Back))
        );
    }

    #[test]
    fn radio_tabs_pick_play_tabs_and_settings_sections() {
        let mut tab = region(HitKind::Toggle, None);
        tab.control_name = Some("navigation_tab".into());
        tab.group_index = Some(1);
        assert_eq!(
            action_for(&view(MenuScreen::Play), &tab),
            Some(MenuAction::Navigate(MenuScreen::Social))
        );
        tab.group_index = Some(8);
        assert_eq!(
            action_for(&view(MenuScreen::Settings), &tab),
            Some(MenuAction::SettingsSection(8))
        );
    }

    #[test]
    fn sliders_split_into_their_settings_values() {
        let mut slider = region(HitKind::Slider, None);
        slider.control_name = Some("gui_scale".to_owned());
        let scale = slider_actions(&slider).unwrap();
        assert_eq!(scale.last(), Some(&MenuAction::SettingsScale(4)));
        slider.control_name = Some("music_volume".to_owned());
        let music = slider_actions(&slider).unwrap();
        assert_eq!(music.len(), usize::from(VOLUME_STEPS));
        assert_eq!(music[0], MenuAction::SettingsVolume(1, 0));
        assert_eq!(music.last(), Some(&MenuAction::SettingsVolume(1, 100)));
        slider.control_name = Some("fov".to_owned());
        assert!(slider_actions(&slider).is_none());
    }

    #[test]
    fn addresses_split_into_the_ip_and_port_boxes() {
        assert_eq!(
            split_address("play.example:19133"),
            ("play.example".into(), "19133".into())
        );
        assert_eq!(split_address("[::1]:19132"), ("::1".into(), "19132".into()));
        assert_eq!(split_address("host"), ("host".into(), "19132".into()));
    }
}
