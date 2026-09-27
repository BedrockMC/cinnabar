use ::protocol::*;
use bytes::BytesMut;
use valentine::bedrock::{codec::BedrockCodec, version::v1_26_44::*};

fn ingredient(key: &str, value: &str) -> CerealizerRecipeIngredientSerializedData {
    CerealizerRecipeIngredientSerializedData {
        descriptor: vec![CerealizerRecipeIngredientSerializedDataDescriptorItem {
            key: key.into(),
            value: value.into(),
        }],
        aux_value: 0,
        stack_size: 1,
    }
}

fn empty() -> CerealizerRecipeIngredientSerializedData {
    CerealizerRecipeIngredientSerializedData::default()
}

fn result() -> Vec<CerealizerNetworkItemInstanceDescriptorSerializedData> {
    vec![CerealizerNetworkItemInstanceDescriptorSerializedData {
        id: 61,
        stacksize: 1,
        auxvalue: 0,
        block_runtime_id: 0,
        user_data_buffer: Vec::new(),
    }]
}

fn catalog() -> RecipeCatalog {
    let ring = (0..9)
        .map(|index| {
            if index == 4 {
                empty()
            } else {
                ingredient("name", "minecraft:cobblestone")
            }
        })
        .collect();
    let mut bytes = BytesMut::new();
    CraftingDataPacket {
        shaped_recipes: vec![
            ShapedRecipePayload {
                recipe_id: "test:furnace".into(),
                width: 3,
                height: 3,
                ingredients: ring,
                results: result(),
                tag: "crafting_table".into(),
                net_id: TypedServerNetIdstructRecipeNetIdTag { raw_id: 3 },
                ..Default::default()
            },
            ShapedRecipePayload {
                recipe_id: "test:stick".into(),
                width: 1,
                height: 2,
                ingredients: vec![
                    ingredient("item_tag", "minecraft:planks"),
                    ingredient("item_tag", "minecraft:planks"),
                ],
                results: result(),
                tag: "crafting_table".into(),
                net_id: TypedServerNetIdstructRecipeNetIdTag { raw_id: 4 },
                ..Default::default()
            },
        ],
        shapeless_recipes: vec![ShapelessRecipePayload {
            recipe_id: "test:dye".into(),
            ingredients: vec![ingredient("name", "minecraft:poppy")],
            results: result(),
            tag: "crafting_table".into(),
            net_id: TypedServerNetIdstructRecipeNetIdTag { raw_id: 5 },
            ..Default::default()
        }],
        clear_recipes: true,
        ..Default::default()
    }
    .encode(&mut bytes)
    .unwrap();
    let mut catalog = RecipeCatalog::default();
    catalog.begin_session(1);
    assert!(catalog.apply(1, 1, &decode_recipe_update(&bytes).unwrap()));
    catalog
}

fn cell(identifier: &str) -> Option<CraftGridItem<'_>> {
    Some(CraftGridItem {
        identifier,
        metadata: 0,
        count: 1,
        plain: true,
    })
}

/// Three-by-three shaped, shapeless and tagged recipes all decode; tags are
/// retained but never match without membership data.
#[test]
fn table_shapeless_and_tagged_recipes_decode_from_the_wire() {
    let catalog = catalog();
    for id in [3, 4, 5] {
        assert!(catalog.recipe(id).is_some(), "recipe {id}");
    }
    let ring: Vec<_> = (0..9)
        .map(|index| {
            (index != 4)
                .then(|| cell("minecraft:cobblestone"))
                .flatten()
        })
        .collect();
    let CraftGridMatch::Unique(furnace) = match_crafting_grid(&catalog, 3, &ring) else {
        panic!("the ring forms one recipe");
    };
    assert_eq!(furnace.network_id(), 3);
    assert_eq!(furnace.output().network_id, 61);
    assert_eq!(furnace.ingredient_counts().count(), 8);

    let CraftGridMatch::Unique(dye) =
        match_crafting_grid(&catalog, 2, &[None, None, None, cell("minecraft:poppy")])
    else {
        panic!("a shapeless recipe matches in any cell");
    };
    assert_eq!(dye.network_id(), 5);

    let planks = [
        cell("minecraft:oak_planks"),
        None,
        cell("minecraft:oak_planks"),
        None,
    ];
    assert!(matches!(
        match_crafting_grid(&catalog, 2, &planks),
        CraftGridMatch::NoMatch
    ));
}
