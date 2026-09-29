//! The play screen's worlds tab over the local-worlds module: its list feeds
//! the tab's cards and a chosen card selects and opens that world.

use protocol::world_control::World;

use super::{LocalWorldCard, MenuRuntime};
use crate::local_worlds::{LocalWorlds, game_mode_label};

impl MenuRuntime {
    /// Mirror the module's world list and forward a pending play choice.
    pub(crate) fn sync_local_worlds(&mut self, worlds: &mut LocalWorlds) {
        let cards = worlds.menu().worlds().iter().map(world_card).collect();
        self.set_local_worlds(cards);
        if let Some(index) = self.take_local_world_request() {
            worlds.input(crate::local_worlds::Input::Select(index));
            worlds.input(crate::local_worlds::Input::Play);
        }
    }
}

fn world_card(world: &World) -> LocalWorldCard {
    LocalWorldCard {
        name: world.name.clone(),
        game_mode: game_mode_label(world.game_mode).to_owned(),
        date: civil_date(world.last_played_unix.max(world.created_unix)),
        // The core does not report world sizes.
        size: String::new(),
    }
}

/// `month/day/year` of a UTC unix time; empty before the epoch.
fn civil_date(unix: i64) -> String {
    if unix <= 0 {
        return String::new();
    }
    // Days-to-civil over 400-year eras (proleptic Gregorian).
    let days = unix / 86_400 + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{month}/{day}/{year}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_render_as_month_day_year() {
        assert_eq!(civil_date(0), "");
        assert_eq!(civil_date(951_782_400), "2/29/2000");
        assert_eq!(civil_date(1_790_553_600), "9/28/2026");
    }

    #[test]
    fn a_chosen_card_selects_the_world_in_the_module() {
        let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
        let mut worlds = LocalWorlds::default();
        menu.sync_local_worlds(&mut worlds);
        assert!(menu.view().local_worlds.is_empty());
        menu.activate(super::super::MenuAction::PlayLocalWorld(0));
        assert_eq!(menu.take_local_world_request(), None);
    }
}
