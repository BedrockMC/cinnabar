use super::*;

impl ActorStore {
    /// Inserts or replaces a profile while preserving roster and retained-skin limits.
    pub(super) fn upsert_profile(&mut self, uuid: [u8; 16], mut profile: PlayerProfile) -> bool {
        if self.players.len() >= self.max_players && !self.players.contains_key(&uuid) {
            return false;
        }
        let previous = self.players.get(&uuid);
        let previous_skin_bytes = previous.map_or(0, |profile| retained_skin_bytes(&profile.skin));
        let retained_without_previous = self
            .retained_player_skin_bytes
            .saturating_sub(previous_skin_bytes);
        let requested_skin_bytes = retained_skin_bytes(&profile.skin);
        let (skin, retained_player_skin_bytes) = retained_without_previous
            .checked_add(requested_skin_bytes)
            .filter(|total| *total <= self.max_player_skin_bytes)
            .map_or_else(
                || {
                    previous.map_or_else(
                        || {
                            (
                                PlayerSkin::Unavailable(
                                    PlayerSkinUnavailable::RetainedBudgetExceeded,
                                ),
                                retained_without_previous,
                            )
                        },
                        |profile| {
                            (
                                profile.skin.clone(),
                                retained_without_previous.saturating_add(previous_skin_bytes),
                            )
                        },
                    )
                },
                |total| (profile.skin.clone(), total),
            );
        self.retained_player_skin_bytes = retained_player_skin_bytes;

        profile.skin = skin;
        self.players.insert(uuid, profile);
        true
    }

    /// Removes a profile and releases its retained skin charge.
    pub(super) fn remove_profile(&mut self, uuid: &[u8; 16]) {
        if let Some(profile) = self.players.remove(uuid) {
            self.retained_player_skin_bytes = self
                .retained_player_skin_bytes
                .saturating_sub(retained_skin_bytes(&profile.skin));
        }
    }
}
