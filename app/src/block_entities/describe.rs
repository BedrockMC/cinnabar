//! Turns a block entity's id, backing block state and NBT into what the renderer draws.

use render::{
    BannerLayer, BannerModel, BannerMount, BeaconModel, BlockEntityKind, ChestModel, ChestPair,
    ChestVariant, CopperAge, Facing, MAX_BANNER_LAYERS, ShulkerModel, SignMount, SkullKind,
    SkullModel, SkullMount, banner_color, pattern_texture, shulker_color_from_block_name,
};
use world::NbtCompound;

use super::{sign_text::SignTextSpec, state::BlockState};

/// Highest world Y a beacon beam is drawn to; the beam stops at the build limit.
const BEAM_TOP: i32 = 320;
const DEFAULT_SIGN_COLOR: i32 = -0x1000000;

/// A block entity's drawing plan before per-frame animation and text resolution.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Template {
    Static(BlockEntityKind),
    Chest(ChestModel),
    Shulker(ShulkerModel),
    Sign {
        mount: SignMount,
        front: Option<SignTextSpec>,
        back: Option<SignTextSpec>,
    },
    EnchantTable,
}

/// Facing from either state spelling: `minecraft:cardinal_direction` or `facing_direction`.
fn facing(state: &BlockState) -> Option<Facing> {
    state
        .text("minecraft:cardinal_direction")
        .and_then(Facing::from_cardinal)
        .or_else(|| {
            state
                .int("facing_direction")
                .and_then(Facing::from_facing_direction)
        })
}

/// Sixteen-step ground rotation in degrees, 0 facing south.
fn ground_rotation(state: &BlockState) -> f32 {
    state
        .int("ground_sign_direction")
        .unwrap_or(0)
        .rem_euclid(16) as f32
        * 22.5
}

fn chest_variant(block_name: &str) -> Option<ChestVariant> {
    let name = block_name.strip_prefix("minecraft:")?;
    let name = name.strip_prefix("waxed_").unwrap_or(name);
    Some(match name {
        "chest" => ChestVariant::Normal,
        "trapped_chest" => ChestVariant::Trapped,
        "copper_chest" => ChestVariant::Copper(CopperAge::Unaffected),
        "exposed_copper_chest" => ChestVariant::Copper(CopperAge::Exposed),
        "weathered_copper_chest" => ChestVariant::Copper(CopperAge::Weathered),
        "oxidized_copper_chest" => ChestVariant::Copper(CopperAge::Oxidized),
        _ => return None,
    })
}

fn chest_pair(position: [i32; 3], nbt: &NbtCompound) -> ChestPair {
    let (Some(pair_x), Some(pair_z)) = (nbt.integer("pairx"), nbt.integer("pairz")) else {
        return ChestPair::Single;
    };
    let (Ok(pair_x), Ok(pair_z)) = (i32::try_from(pair_x), i32::try_from(pair_z)) else {
        return ChestPair::Single;
    };
    // Without an explicit lead flag the lower coordinate leads, so exactly one half draws.
    let lead = nbt
        .boolean("pairlead")
        .unwrap_or((position[0], position[2]) < (pair_x, pair_z));
    if lead {
        ChestPair::Lead {
            partner: [pair_x, position[1], pair_z],
        }
    } else {
        ChestPair::Follower
    }
}

fn banner(block_name: &str, state: &BlockState, nbt: &NbtCompound) -> Option<Template> {
    let mount = match block_name.strip_prefix("minecraft:")? {
        "standing_banner" => BannerMount::Standing {
            rotation_degrees: ground_rotation(state),
        },
        "wall_banner" => BannerMount::Wall(facing(state)?),
        _ => return None,
    };
    let layers = nbt
        .list("Patterns")
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let world::NbtValue::Compound(entry) = entry else {
                return None;
            };
            Some(BannerLayer {
                pattern: pattern_texture(entry.string("Pattern")?)?,
                color: banner_color(entry.integer("Color")?),
            })
        })
        .take(MAX_BANNER_LAYERS)
        .collect();
    Some(Template::Static(BlockEntityKind::Banner(BannerModel {
        mount,
        base: banner_color(nbt.integer("Base").unwrap_or(15)),
        layers,
    })))
}

fn sign_mount(block_name: &str, state: &BlockState) -> Option<SignMount> {
    let name = block_name.strip_prefix("minecraft:")?;
    if name.ends_with("hanging_sign") {
        return Some(if state.int("attached_bit") == Some(1) {
            SignMount::Hanging {
                rotation_degrees: ground_rotation(state),
            }
        } else {
            SignMount::HangingWall(facing(state)?)
        });
    }
    if name.ends_with("wall_sign") {
        return Some(SignMount::Wall(facing(state)?));
    }
    name.ends_with("standing_sign")
        .then(|| SignMount::Standing {
            rotation_degrees: ground_rotation(state),
        })
}

fn sign_face(face: &NbtCompound) -> Option<SignTextSpec> {
    let spec = SignTextSpec {
        text: face.string("Text")?.to_owned(),
        color_argb: face
            .integer("SignTextColor")
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(DEFAULT_SIGN_COLOR),
        glowing: face.boolean("IgnoreLighting").unwrap_or(false),
        hide_glow_outline: face.boolean("HideGlowOutline").unwrap_or(false),
    };
    spec.is_visible().then_some(spec)
}

fn sign(block_name: &str, state: &BlockState, nbt: &NbtCompound) -> Option<Template> {
    let mount = sign_mount(block_name, state)?;
    let (front, back) = match (nbt.compound("FrontText"), nbt.compound("BackText")) {
        (None, None) => (
            // Pre-1.19.80 signs carry one face's text at the root.
            sign_face(nbt),
            None,
        ),
        (front, back) => (front.and_then(sign_face), back.and_then(sign_face)),
    };
    (front.is_some() || back.is_some()).then_some(Template::Sign { mount, front, back })
}

/// Plans the draw for one block entity, or `None` when nothing should be drawn.
pub(super) fn describe(
    id: &str,
    block_name: &str,
    state: &BlockState,
    nbt: &NbtCompound,
    position: [i32; 3],
) -> Option<Template> {
    match id {
        "Chest" => Some(Template::Chest(ChestModel {
            variant: chest_variant(block_name)?,
            facing: facing(state).unwrap_or(Facing::North),
            pair: chest_pair(position, nbt),
            lid: 0.0,
        })),
        "EnderChest" => Some(Template::Chest(ChestModel {
            variant: ChestVariant::Ender,
            facing: facing(state).unwrap_or(Facing::North),
            pair: ChestPair::Single,
            lid: 0.0,
        })),
        "ShulkerBox" => Some(Template::Shulker(ShulkerModel {
            color: shulker_color_from_block_name(block_name)?,
            facing: nbt
                .integer("facing")
                .and_then(|value| u8::try_from(value).ok())
                .filter(|value| *value < 6)
                .unwrap_or(1),
            open: 0.0,
        })),
        "Skull" => {
            let kind = SkullKind::from_nbt(nbt.integer("SkullType")?)?;
            let mount = match state
                .int("facing_direction")
                .and_then(Facing::from_facing_direction)
            {
                Some(wall) => SkullMount::Wall(wall),
                None => SkullMount::Floor {
                    rotation_degrees: nbt.float("Rotation").unwrap_or(0.0),
                },
            };
            Some(Template::Static(BlockEntityKind::Skull(SkullModel {
                kind,
                mount,
            })))
        }
        "Banner" => banner(block_name, state, nbt),
        "Sign" | "HangingSign" => sign(block_name, state, nbt),
        "EnchantTable" => Some(Template::EnchantTable),
        "Lectern" => nbt.boolean("hasBook").unwrap_or(false).then(|| {
            Template::Static(BlockEntityKind::Lectern {
                facing_yaw_degrees: facing(state).unwrap_or(Facing::North).yaw_degrees(),
            })
        }),
        "Bell" => Some(Template::Static(BlockEntityKind::Bell)),
        "Beacon" => {
            let height = (BEAM_TOP - position[1] - 1).max(0) as u32;
            (nbt.integer("Levels").unwrap_or(0) > 0 && height > 0).then_some(Template::Static(
                BlockEntityKind::Beacon(BeaconModel {
                    height,
                    tint: [1.0; 3],
                }),
            ))
        }
        "EndPortal" => Some(Template::Static(BlockEntityKind::EndPortal)),
        "EndGateway" => Some(Template::Static(BlockEntityKind::EndGateway)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nbt(build: impl FnOnce(&mut Vec<u8>)) -> NbtCompound {
        let mut bytes = vec![10, 0];
        build(&mut bytes);
        bytes.push(0);
        world::BlockEntityNbt::decode_prefix(&bytes)
            .unwrap()
            .0
            .parse()
            .unwrap()
    }

    fn int_tag(out: &mut Vec<u8>, key: &str, zigzag: u8) {
        out.push(3);
        out.push(key.len() as u8);
        out.extend_from_slice(key.as_bytes());
        out.push(zigzag);
    }

    fn state(json: &str) -> BlockState {
        BlockState::parse(json)
    }

    #[test]
    fn chest_pair_leads_by_flag_or_lower_coordinate() {
        // pairx = 5 (zigzag 10), pairz = 0.
        let plain = nbt(|out| {
            int_tag(out, "pairx", 10);
            int_tag(out, "pairz", 0);
        });
        assert_eq!(
            chest_pair([4, 64, 0], &plain),
            ChestPair::Lead {
                partner: [5, 64, 0]
            }
        );
        assert_eq!(chest_pair([6, 64, 0], &plain), ChestPair::Follower);
        assert_eq!(chest_pair([4, 64, 0], &nbt(|_| {})), ChestPair::Single);
    }

    #[test]
    fn chest_variants_and_facing_resolve_from_the_block() {
        let template = describe(
            "Chest",
            "minecraft:waxed_exposed_copper_chest",
            &state(r#"{"minecraft:cardinal_direction":{"type":"string","value":"east"}}"#),
            &nbt(|_| {}),
            [0, 0, 0],
        );
        let Some(Template::Chest(model)) = template else {
            panic!("chest expected");
        };
        assert_eq!(model.variant, ChestVariant::Copper(CopperAge::Exposed));
        assert_eq!(model.facing, Facing::East);
        assert!(
            describe(
                "Chest",
                "minecraft:stone",
                &BlockState::default(),
                &nbt(|_| {}),
                [0; 3]
            )
            .is_none()
        );
    }

    #[test]
    fn sign_text_needs_visible_characters_on_some_face() {
        let empty = nbt(|_| {});
        let standing = state(r#"{"ground_sign_direction":4}"#);
        assert!(describe("Sign", "minecraft:standing_sign", &standing, &empty, [0; 3]).is_none());
        let text = nbt(|out| {
            out.push(8);
            out.push(4);
            out.extend_from_slice(b"Text");
            out.push(2);
            out.extend_from_slice(b"hi");
        });
        let Some(Template::Sign { mount, front, back }) =
            describe("Sign", "minecraft:standing_sign", &standing, &text, [0; 3])
        else {
            panic!("legacy root text draws on the front");
        };
        assert_eq!(
            mount,
            SignMount::Standing {
                rotation_degrees: 90.0
            }
        );
        assert_eq!(front.unwrap().text, "hi");
        assert!(back.is_none());
    }

    #[test]
    fn ids_without_a_renderer_draw_nothing() {
        assert!(
            describe(
                "Furnace",
                "minecraft:furnace",
                &BlockState::default(),
                &nbt(|_| {}),
                [0; 3]
            )
            .is_none()
        );
    }
}
