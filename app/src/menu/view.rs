//! The menu's presented data: saved servers, catalog cards, and the per-frame
//! view the renderers draw from.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::{MenuAction, MenuDialog, MenuField, MenuScreen, MenuServerTab, auth::AuthState};
use crate::ui_runtime::presentation::IconRef;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedServer {
    pub(crate) name: String,
    pub(crate) address: String,
    #[serde(default)]
    pub(crate) favorite: bool,
    #[serde(default)]
    pub(crate) last_joined_unix: u64,
}

/// One local world for the play screen's worlds tab, supplied by the local
/// worlds module through [`super::MenuRuntime::set_local_worlds`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LocalWorldCard {
    pub(crate) name: String,
    pub(crate) game_mode: String,
    pub(crate) date: String,
    pub(crate) size: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct MenuServerCard {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) caption: String,
    #[serde(default)]
    pub(crate) image_path: String,
    #[serde(skip)]
    pub(crate) icon: Option<IconRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub(crate) struct MenuRealmCard {
    pub(crate) name: String,
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) address: String,
    #[serde(default)]
    pub(crate) owner: String,
    #[serde(default)]
    pub(crate) online_players: u32,
    #[serde(default)]
    pub(crate) max_players: u32,
    #[serde(default)]
    pub(crate) days_left: i32,
    #[serde(default)]
    pub(crate) expired: bool,
    /// Joined as a member rather than owned.
    #[serde(default)]
    pub(crate) member: bool,
}

/// A featured server's info-panel details; artwork is a local cached path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ServerDetails {
    pub(crate) description: String,
    pub(crate) news_title: String,
    pub(crate) news: String,
    pub(crate) screenshots: Vec<String>,
    pub(crate) games: Vec<MenuGameCard>,
}

/// One game a featured server advertises.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MenuGameCard {
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) description: String,
    pub(crate) image_path: String,
}

/// The signed-in profile as the start screen shows it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MenuProfile {
    pub(crate) gamertag: String,
    pub(crate) picture_path: String,
}

/// Service feed data beyond the catalog cards: featured-server details keyed
/// by address, the profile, and the featured server the info panel shows.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MenuFeeds {
    pub(crate) details: HashMap<String, ServerDetails>,
    pub(crate) profile: MenuProfile,
    pub(crate) selected_featured: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MenuFriendCard {
    pub(crate) gamertag: String,
    pub(crate) world_name: String,
    pub(crate) members: String,
    pub(crate) xuid: String,
}

#[derive(Clone, Debug)]
pub(crate) struct MenuView {
    pub(crate) visible: bool,
    pub(crate) screen: MenuScreen,
    pub(crate) focused_action: Option<MenuAction>,
    pub(crate) hovered: Option<MenuAction>,
    pub(crate) pressed: Option<MenuAction>,
    pub(crate) server_tab: MenuServerTab,
    pub(crate) dialog: Option<MenuDialog>,
    pub(crate) field: Option<MenuField>,
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) message: Option<String>,
    pub(crate) gui_scale: u8,
    pub(crate) display_name: String,
    pub(crate) servers: Vec<SavedServer>,
    pub(crate) featured: Vec<MenuServerCard>,
    pub(crate) gatherings: Vec<MenuServerCard>,
    pub(crate) realms: Vec<MenuRealmCard>,
    pub(crate) friends: Vec<MenuFriendCard>,
    pub(crate) featured_icon: Option<IconRef>,
    pub(crate) gathering_icon: Option<IconRef>,
    pub(crate) realm_icon: Option<IconRef>,
    pub(crate) friend_icon: Option<IconRef>,
    pub(crate) saved_icon: Option<IconRef>,
    pub(crate) profile_icon: Option<IconRef>,
    pub(crate) catalog_loading: bool,
    pub(crate) catalog_message: Option<String>,
    pub(crate) auth_state: AuthState,
    pub(crate) connecting: bool,
    pub(crate) settings_section: u8,
    /// Why the last session ended, shown until acknowledged.
    pub(crate) disconnect_message: Option<String>,
    /// The saved server the add screen is editing.
    pub(crate) editing: Option<usize>,
    pub(crate) local_worlds: Vec<LocalWorldCard>,
    pub(crate) volumes: super::settings_values::Volumes,
    pub(crate) feeds: MenuFeeds,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct CatalogFile {
    #[serde(default)]
    pub(super) featured: Vec<MenuServerCard>,
    #[serde(default)]
    pub(super) gatherings: Vec<MenuServerCard>,
    #[serde(default)]
    pub(super) realms: Vec<MenuRealmCard>,
    #[serde(default)]
    pub(super) friends: Vec<CatalogFriend>,
    #[serde(default)]
    pub(super) errors: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct CatalogFriend {
    pub(super) gamertag: String,
    pub(super) world_name: String,
    pub(super) xuid: String,
    pub(super) members: i32,
    pub(super) max_members: i32,
}

impl From<CatalogFriend> for MenuFriendCard {
    fn from(friend: CatalogFriend) -> Self {
        let members = if friend.max_members > 0 {
            format!("{}/{} players", friend.members, friend.max_members)
        } else {
            format!("{} players", friend.members)
        };
        Self {
            gamertag: friend.gamertag,
            world_name: friend.world_name,
            members,
            xuid: friend.xuid,
        }
    }
}
