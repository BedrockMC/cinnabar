//! Health, armor, hunger, mount-health and air rows.

use assets::HudTextureRole;

use ui::{UiNode, UiNodeId, UiVisual};

use super::{
    HudFrame, HudLayout, UiPresentationError, UiRuntime, rect,
    pinned::{HOTBAR_WIDTH, MAX_HEART_ROWS, MAX_MOUNT_HEARTS, damage_flash_phase, heart_role},
    status_motion::{heart_lift, hunger_shake_offset, hunger_shakes},
};

impl HudLayout<'_> {
    pub(super) fn health_rows(
        &mut self,
        runtime: &UiRuntime,
        frame: &HudFrame,
        now_tick: Option<u64>,
    ) -> Result<(), UiPresentationError> {
        let Some(health) = runtime.hud().health() else {
            return Ok(());
        };
        let scale = u32::from(health.scale());
        // Half-heart units on the reference 20-point scale.
        let current = u32::from(health.current()).div_ceil(scale.max(1));
        let maximum = u32::from(health.maximum()) / scale.max(1);
        let absorption = runtime
            .hud()
            .absorption()
            .map(|stat| u32::from(stat.current()).div_ceil(u32::from(stat.scale()).max(1)))
            .unwrap_or(0);
        let health_hearts = maximum.div_ceil(2).min(u32::from(MAX_HEART_ROWS) * 10);
        let absorption_hearts = absorption.div_ceil(2).min(20);
        let total_hearts = (health_hearts + absorption_hearts).max(1);
        let rows = total_hearts.div_ceil(10).max(1) as u16;
        let row_height = (10 - (rows.saturating_sub(2))).max(3) as f32;

        let variant = runtime.gameplay_hud().heart_variant(now_tick);
        let flash = damage_flash_phase(runtime.last_health_drop_millis(), frame.now_millis);
        let g = self.geometry;
        let base = [(g.gui_width - HOTBAR_WIDTH) / 2.0, g.gui_height - 39.0];
        let tick = now_tick.unwrap_or(frame.now_millis / 50);
        let regenerating = runtime.gameplay_hud().regeneration_active(now_tick);
        let low_health_halves = current + absorption;
        for index in 0..total_hearts {
            let row = index / 10;
            let column = index % 10;
            let lift = heart_lift(index, health_hearts, low_health_halves, regenerating, tick);
            let position = [
                base[0] + column as f32 * 8.0,
                base[1] - row as f32 * row_height - lift,
            ];
            self.sprite_gui(HudTextureRole::HeartBackground, position, [255; 4])?;
            let foreground = if index < health_hearts {
                let filled = current.saturating_sub(index * 2);
                heart_role(variant, flash, filled)
            } else {
                let filled = absorption.saturating_sub((index - health_hearts) * 2);
                match filled {
                    0 => None,
                    1 => Some(HudTextureRole::AbsorptionHeartHalf),
                    _ => Some(HudTextureRole::AbsorptionHeartFull),
                }
            };
            if let Some(role) = foreground {
                let hardcore = frame
                    .hardcore_hearts
                    .filter(|_| runtime.gameplay_hud().hardcore())
                    .and_then(|hearts| hearts.sprite(role));
                match hardcore {
                    Some((page, uv)) => self.extra_sprite_gui(page, uv, position)?,
                    None => self.sprite_gui(role, position, [255; 4])?,
                }
            }
        }
        Ok(())
    }

    /// A 9x9 GUI px sprite from the extras page.
    fn extra_sprite_gui(
        &mut self,
        page: u16,
        uv: [u16; 4],
        gui: [f32; 2],
    ) -> Result<(), UiPresentationError> {
        let g = self.geometry;
        let [x, y] = g.logical(gui);
        let node = UiNode::new(
            UiNodeId::new(*self.next_id),
            None,
            rect(x, y, x + 9.0 * g.scale, y + 9.0 * g.scale)?,
        )
        .with_visual(UiVisual::Sprite {
            texture_page: page,
            uv,
            color: [255; 4],
        });
        self.nodes.push(node);
        *self.next_id = self.next_id.saturating_add(1);
        Ok(())
    }

    /// Armor sits one row above the highest heart row and appears only while
    /// the authoritative equipped armor total is nonzero.
    pub(super) fn armor_row(&mut self, runtime: &UiRuntime) -> Result<(), UiPresentationError> {
        let Some(armor) = runtime.hud().armor() else {
            return Ok(());
        };
        let points = u32::from(armor.current()).div_ceil(u32::from(armor.scale()).max(1));
        if points == 0 {
            return Ok(());
        }
        let Some(health) = runtime.hud().health() else {
            return Ok(());
        };
        let scale = u32::from(health.scale()).max(1);
        let maximum = u32::from(health.maximum()) / scale;
        let absorption = runtime
            .hud()
            .absorption()
            .map(|stat| u32::from(stat.current()).div_ceil(u32::from(stat.scale()).max(1)))
            .unwrap_or(0);
        let hearts = (maximum.div_ceil(2).min(u32::from(MAX_HEART_ROWS) * 10)
            + absorption.div_ceil(2).min(20))
        .max(1);
        let rows = hearts.div_ceil(10).max(1) as u16;
        let row_height = (10 - (rows.saturating_sub(2))).max(3) as f32;
        let g = self.geometry;
        let y = g.gui_height - 39.0 - (rows.saturating_sub(1)) as f32 * row_height - 10.0;
        let x = (g.gui_width - HOTBAR_WIDTH) / 2.0;
        for index in 0..10u32 {
            let position = [x + index as f32 * 8.0, y];
            let remaining = points.saturating_sub(index * 2);
            let role = match remaining {
                0 => HudTextureRole::ArmorEmpty,
                1 => HudTextureRole::ArmorHalf,
                _ => HudTextureRole::ArmorFull,
            };
            self.sprite_gui(role, position, [255; 4])?;
        }
        Ok(())
    }

    pub(super) fn hunger_row(
        &mut self,
        runtime: &UiRuntime,
        now_tick: Option<u64>,
    ) -> Result<(), UiPresentationError> {
        let Some(hunger) = runtime.hud().hunger() else {
            return Ok(());
        };
        let scale = u32::from(hunger.scale()).max(1);
        let current = u32::from(hunger.current()).div_ceil(scale);
        let effect = runtime.gameplay_hud().hunger_effect_active(now_tick);
        let (background, full, half) = if effect {
            (
                HudTextureRole::HungerEffectBackground,
                HudTextureRole::HungerEffectFull,
                HudTextureRole::HungerEffectHalf,
            )
        } else {
            (
                HudTextureRole::HungerBackground,
                HudTextureRole::HungerFull,
                HudTextureRole::HungerHalf,
            )
        };
        let g = self.geometry;
        let right = (g.gui_width + HOTBAR_WIDTH) / 2.0;
        let tick = now_tick.unwrap_or(0);
        // Without a server clock there is no tick to pulse on, so no shake.
        let shaking = now_tick.is_some()
            && hunger_shakes(runtime.gameplay_hud().saturation_empty(), current, tick);
        for index in 0..10u32 {
            let shake = if shaking {
                hunger_shake_offset(index, tick)
            } else {
                0.0
            };
            let position = [
                right - index as f32 * 8.0 - 9.0,
                g.gui_height - 39.0 + shake,
            ];
            self.sprite_gui(background, position, [255; 4])?;
            let remaining = current.saturating_sub(index * 2);
            let role = match remaining {
                0 => None,
                1 => Some(half),
                _ => Some(full),
            };
            if let Some(role) = role {
                self.sprite_gui(role, position, [255; 4])?;
            }
        }
        Ok(())
    }

    /// Mount hearts replace the hunger row while riding, right-aligned like
    /// the reference, capped at 30 hearts across up to three rows.
    pub(super) fn mount_health_rows(
        &mut self,
        frame: &HudFrame,
    ) -> Result<(), UiPresentationError> {
        let Some((current, maximum)) = frame.mount_health else {
            return Ok(());
        };
        let hearts = ((maximum + 0.5) / 2.0) as u16;
        let hearts = hearts.clamp(1, MAX_MOUNT_HEARTS);
        let filled_halves = current.clamp(0.0, maximum).ceil() as u32;
        let g = self.geometry;
        let right = (g.gui_width + HOTBAR_WIDTH) / 2.0;
        for index in 0..u32::from(hearts) {
            let row = index / 10;
            let column = index % 10;
            let position = [
                right - (column as f32 % 10.0) * 8.0 - 9.0,
                g.gui_height - 39.0 - row as f32 * 10.0,
            ];
            self.sprite_gui(HudTextureRole::HeartBackground, position, [255; 4])?;
            let remaining = filled_halves.saturating_sub(index * 2);
            let role = match remaining {
                0 => None,
                1 => Some(HudTextureRole::MountHeartHalf),
                _ => Some(HudTextureRole::MountHeartFull),
            };
            if let Some(role) = role {
                self.sprite_gui(role, position, [255; 4])?;
            }
        }
        Ok(())
    }

    /// Air bubbles above the hunger column, visible only while submerged
    /// (air below its maximum), with the reference's popping tail.
    pub(super) fn air_row(&mut self, runtime: &UiRuntime) -> Result<(), UiPresentationError> {
        let Some(air) = runtime.hud().air() else {
            return Ok(());
        };
        let current = u32::from(air.current());
        let maximum = u32::from(air.maximum()).max(1);
        if current >= maximum {
            return Ok(());
        }
        let full = (current.saturating_sub(2) * 10).div_ceil(maximum);
        let popping = (current * 10).div_ceil(maximum).saturating_sub(full);
        let g = self.geometry;
        let right = (g.gui_width + HOTBAR_WIDTH) / 2.0;
        for index in 0..(full + popping).min(10) {
            let role = if index < full {
                HudTextureRole::BubbleFull
            } else {
                HudTextureRole::BubblePop
            };
            self.sprite_gui(
                role,
                [right - index as f32 * 8.0 - 9.0, g.gui_height - 49.0],
                [255; 4],
            )?;
        }
        Ok(())
    }
}
