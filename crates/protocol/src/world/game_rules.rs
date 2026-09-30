//! The world rules the client reads from StartGame and GameRulesChanged.

use super::{GameData, GameRule, GameRuleRuleValue};

/// Reads the authoritative `doDaylightCycle` switch from a rule list.
///
/// 1.26.40 collapses the 1.26.30 `GameRuleI32` / `GameRuleVarint` pair (and
/// their separate `type_` discriminants) into one `GameRule` whose value is a
/// tagged union, so the redundant "declared type matches the value arm" check
/// the old modelling required is gone: a non-boolean rule simply cannot decode
/// into `GameRuleRuleValue::Bool`.
pub(super) fn daylight_cycle_rule_update(rules: &[GameRule]) -> Option<bool> {
    bool_rule(rules, "dodaylightcycle")
}

fn bool_rule(rules: &[GameRule], name: &str) -> Option<bool> {
    rules.iter().find_map(|rule| {
        if rule.rule_name.eq_ignore_ascii_case(name)
            && let GameRuleRuleValue::Bool(enabled) = &rule.rule_value
        {
            Some(*enabled)
        } else {
            None
        }
    })
}

pub(super) fn hud_rules(rules: &[GameRule]) -> crate::HudRules {
    crate::HudRules {
        show_coordinates: bool_rule(rules, "showcoordinates"),
        show_days_played: bool_rule(rules, "showdaysplayed"),
    }
}

impl crate::HudRules {
    /// StartGame's HUD rules; an absent rule reads as off, its vanilla default.
    #[must_use]
    pub fn from_game_data(game_data: &GameData) -> Self {
        let rules = hud_rules(&game_data.start_game.settings.rule_data.rules_list);
        Self {
            show_coordinates: Some(rules.show_coordinates.unwrap_or(false)),
            show_days_played: Some(rules.show_days_played.unwrap_or(false)),
        }
    }
}
