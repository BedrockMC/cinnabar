//! The vanilla play screen's bindings: Worlds (local worlds and Realms), Friends
//! (joinable friend worlds and member Realms) and Servers (saved servers, and
//! the featured list of servers then gatherings with the selected one's info
//! panel), plus how its world and server presses map back to menu actions.

use json_ui::{CollectionItem, DataSource, HitRegion, Scalar};

use crate::menu::{MenuAction, MenuRealmCard, MenuScreen, MenuServerCard, MenuView};

const FEATURED: &str = "third_party_server_network_worlds";
const PERSONAL_REALMS: &str = "personal_realms";
const FRIEND_REALMS: &str = "friends_realms";

fn text(value: impl Into<String>) -> Scalar {
    Scalar::Text(value.into())
}

fn flag(data: &mut DataSource, name: &str, on: bool) {
    data.set_global(name, Scalar::Bool(on));
}

/// Featured servers followed by gatherings, as the Servers tab lists them.
fn featured(view: &MenuView) -> impl Iterator<Item = &MenuServerCard> {
    view.featured.iter().chain(view.gatherings.iter())
}

pub(super) fn bind(view: &MenuView, data: &mut DataSource) {
    let tab = match view.screen {
        MenuScreen::Social => 1,
        MenuScreen::Servers => 2,
        _ => 0,
    };
    data.select_radio("navigation_tab", tab);
    for name in [
        "#is_network_available_and_multiplayer_visible",
        "#friends_grid_visible",
        "#servers_grid_visible",
        "#featured_servers_visible",
        "#featured_servers_visible_and_available",
        "#realms_grids_visible",
        "#local_worlds_visible",
        "#is_additional_server_label_visible",
    ] {
        flag(data, name, true);
    }
    local_worlds(view, data);
    network_worlds(view, data);
    featured_servers(view, data);
    realms(view, data);
}

fn local_worlds(view: &MenuView, data: &mut DataSource) {
    let worlds = view
        .local_worlds
        .iter()
        .map(|world| {
            CollectionItem::default()
                .with("#local_world_name", text(world.name.clone()))
                .with("#local_world_game_mode", text(world.game_mode.clone()))
                .with("#local_world_date", text(world.date.clone()))
                .with("#local_worldfile_size", text(world.size.clone()))
        })
        .collect::<Vec<_>>();
    data.set_global("#world_item_count", text(worlds.len().to_string()));
    data.set_collection("local_worlds", worlds);
}

fn network_worlds(view: &MenuView, data: &mut DataSource) {
    let network = |header: &str, details: &str, players: &str| {
        CollectionItem::default()
            .with("#network_world_header", text(header))
            .with("#network_world_details", text(details))
            .with("#network_world_player_count", text(players))
            .with("#network_world_button_enabled", Scalar::Bool(true))
            .with("#game_online", Scalar::Bool(true))
    };
    let friends = view
        .friends
        .iter()
        .map(|friend| network(&friend.world_name, &friend.gamertag, &friend.members))
        .collect::<Vec<_>>();
    flag(data, "#no_friends_grid_message_visible", friends.is_empty());
    data.set_global("#friend_world_item_count", text(friends.len().to_string()));
    data.set_collection("friends_network_worlds", friends);
    let saved = view
        .servers
        .iter()
        .map(|server| network(&server.name, &server.address, ""))
        .collect::<Vec<_>>();
    data.set_global("#server_world_item_count", text(saved.len().to_string()));
    data.set_collection("servers_network_worlds", saved);
}

fn featured_servers(view: &MenuView, data: &mut DataSource) {
    let items = featured(view)
        .map(|server| {
            CollectionItem::default()
                .with("#third_party_server_name", text(server.name.clone()))
                .with("#third_party_server_message", text(server.caption.clone()))
                .with(
                    "#third_party_server_logo_texture_path",
                    text(server.image_path.clone()),
                )
                .with("#is_server_info_available_collection", Scalar::Bool(true))
        })
        .collect();
    data.set_collection(FEATURED, items);
    let selected = view
        .feeds
        .selected_featured
        .and_then(|index| featured(view).nth(index));
    flag(data, "#is_server_info_available", selected.is_some());
    let Some(server) = selected else {
        return;
    };
    data.set_global("#info_third_party_server_name", text(server.name.clone()));
    data.set_global(
        "#info_third_party_server_logo_texture_path",
        text(server.image_path.clone()),
    );
    flag(
        data,
        "#info_third_party_screenshot_visible",
        !server.image_path.is_empty(),
    );
    let Some(details) = view.feeds.details.get(&server.address) else {
        return;
    };
    flag(
        data,
        "#server_has_description",
        !details.description.is_empty(),
    );
    data.set_global("#description_label", text(details.description.clone()));
    // Long text opens collapsed behind its "read more" toggle.
    flag(data, "#description_is_read_more", true);
    flag(data, "#server_has_news", !details.news.is_empty());
    data.set_global("#news_text", text(details.news.clone()));
    data.set_global("#news_label", text(details.news_title.clone()));
    flag(data, "#news_is_read_more", true);
    let screenshots = details
        .screenshots
        .iter()
        .enumerate()
        .map(|(index, path)| {
            CollectionItem::default()
                .with("#screenshot_texture", text(path.clone()))
                .with("#this_screenshot_selected", Scalar::Bool(index == 0))
        })
        .collect::<Vec<_>>();
    flag(data, "#server_has_screenshots", !screenshots.is_empty());
    data.set_global(
        "#screenshot_collection_length",
        Scalar::Num(screenshots.len() as f64),
    );
    data.set_collection("server_screenshot_collection", screenshots);
    let games = details
        .games
        .iter()
        .map(|game| {
            CollectionItem::default()
                .with("#available_game_title", text(game.title.clone()))
                .with("#available_game_subtitle", text(game.subtitle.clone()))
                .with(
                    "#available_game_description",
                    text(game.description.clone()),
                )
                .with("#available_game_image", text(game.image_path.clone()))
                .with(
                    "#available_game_image_visible",
                    Scalar::Bool(!game.image_path.is_empty()),
                )
        })
        .collect::<Vec<_>>();
    flag(data, "#server_has_games", !games.is_empty());
    data.set_global("#games_collection_length", Scalar::Num(games.len() as f64));
    data.set_collection("server_games_collection", games);
}

fn realms(view: &MenuView, data: &mut DataSource) {
    let item = |realm: &MenuRealmCard| {
        let open = realm.state.eq_ignore_ascii_case("open") && !realm.expired;
        let players = if realm.max_players > 0 {
            format!("{}/{}", realm.online_players, realm.max_players)
        } else {
            realm.online_players.to_string()
        };
        let details = if realm.member {
            realm.owner.clone()
        } else {
            realm.state.clone()
        };
        CollectionItem::default()
            .with("#realms_world_header", text(realm.name.clone()))
            .with("#realms_world_details", text(details))
            .with("#realms_world_player_count", text(players))
            .with("#realms_game_online", Scalar::Bool(open))
            .with(
                "#realms_game_offline",
                Scalar::Bool(!open && !realm.expired),
            )
            .with("#realms_game_unavailable", Scalar::Bool(realm.expired))
            .with(
                "#realms_world_expiry_notification_visible",
                Scalar::Bool(!realm.member && (realm.expired || realm.days_left <= 7)),
            )
    };
    let personal = view
        .realms
        .iter()
        .filter(|realm| !realm.member)
        .map(item)
        .collect::<Vec<_>>();
    let friends = view
        .realms
        .iter()
        .filter(|realm| realm.member)
        .map(item)
        .collect::<Vec<_>>();
    flag(data, "#personal_realms_grid_visible", !personal.is_empty());
    flag(data, "#friends_realms_visible", !friends.is_empty());
    flag(data, "#joinable_realms_panel_visible", !friends.is_empty());
    data.set_collection(PERSONAL_REALMS, personal);
    data.set_collection(FRIEND_REALMS, friends);
}

/// Joining the featured-list entry at `index` (servers, then gatherings).
pub(super) fn play_featured(view: &MenuView, index: usize) -> Option<MenuAction> {
    if index < view.featured.len() {
        return Some(MenuAction::PlayFeatured(index));
    }
    let gathering = index - view.featured.len();
    (gathering < view.gatherings.len()).then_some(MenuAction::PlayGathering(gathering))
}

/// The action for a press on the Servers tab's featured list or info panel.
pub(super) fn featured_action(view: &MenuView, region: &HitRegion) -> Option<MenuAction> {
    let index = match region.collection.as_deref() {
        Some(FEATURED) => region.collection_index?,
        _ => view.feeds.selected_featured?,
    };
    play_featured(view, index)
}

/// The Realms entry a grid press starts, as an index into all realms.
pub(super) fn realm_action(view: &MenuView, region: &HitRegion) -> Option<MenuAction> {
    let index = region.collection_index?;
    let member = region.collection.as_deref() == Some(FRIEND_REALMS);
    view.realms
        .iter()
        .enumerate()
        .filter(|(_, realm)| realm.member == member)
        .nth(index)
        .map(|(overall, _)| MenuAction::PlayRealm(overall))
}

/// Selecting a featured-list entry shows it in the info panel.
pub(super) fn is_featured(region: &HitRegion) -> bool {
    region.collection.as_deref() == Some(FEATURED)
}

#[cfg(test)]
mod tests {
    use json_ui::{HitKind, RectOut};

    use super::*;
    use crate::menu::MenuRuntime;

    fn card(name: &str) -> MenuServerCard {
        MenuServerCard {
            name: name.to_owned(),
            address: format!("{name}.test:19132"),
            caption: String::new(),
            image_path: String::new(),
            icon: None,
        }
    }

    fn realm(name: &str, member: bool) -> MenuRealmCard {
        MenuRealmCard {
            name: name.to_owned(),
            state: "OPEN".to_owned(),
            target: String::new(),
            address: String::new(),
            owner: String::new(),
            online_players: 0,
            max_players: 10,
            days_left: 30,
            expired: false,
            member,
        }
    }

    fn press(collection: Option<&str>, index: Option<usize>) -> HitRegion {
        let rect = RectOut {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        };
        HitRegion {
            key: "/screen/item".to_owned(),
            name: "item".to_owned(),
            kind: HitKind::Button,
            rect,
            clip: rect,
            layer: 0,
            order: 0,
            pressed: None,
            control_name: None,
            collection_index: index,
            collection: collection.map(str::to_owned),
            enabled: true,
            checked: None,
            max_length: None,
            group_index: None,
            renderer: None,
        }
    }

    #[test]
    fn the_featured_list_runs_servers_then_gatherings() {
        let mut view = MenuRuntime::new(true, 2, "Steve".to_owned()).view();
        view.featured = vec![card("a")];
        view.gatherings = vec![card("g")];
        assert_eq!(play_featured(&view, 0), Some(MenuAction::PlayFeatured(0)));
        assert_eq!(play_featured(&view, 1), Some(MenuAction::PlayGathering(0)));
        assert_eq!(play_featured(&view, 2), None);
        // The info panel's join button joins the selected entry.
        view.feeds.selected_featured = Some(1);
        assert_eq!(
            featured_action(&view, &press(None, None)),
            Some(MenuAction::PlayGathering(0))
        );
    }

    #[test]
    fn realm_grids_index_into_all_realms() {
        let mut view = MenuRuntime::new(true, 2, "Steve".to_owned()).view();
        view.realms = vec![
            realm("mine", false),
            realm("theirs", true),
            realm("ours", false),
        ];
        assert_eq!(
            realm_action(&view, &press(Some(FRIEND_REALMS), Some(0))),
            Some(MenuAction::PlayRealm(1))
        );
        assert_eq!(
            realm_action(&view, &press(Some(PERSONAL_REALMS), Some(1))),
            Some(MenuAction::PlayRealm(2))
        );
    }
}
