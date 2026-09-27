use std::sync::Arc;

use super::super::budget::Credits;
use super::super::model::{
    Batch, Ingredient, MAX_INGREDIENTS, Output, Recipe, RecipeUpdate, Record,
};
use super::*;

fn named(name: &str) -> Option<Ingredient> {
    Some(Ingredient {
        name: name.into(),
        tag: false,
        aux: 0,
        count: 1,
    })
}

fn recipe(width: u8, height: u8, shapeless: bool, cells: &[Option<&str>]) -> Recipe {
    let mut ingredients: [Option<Ingredient>; MAX_INGREDIENTS] = std::array::from_fn(|_| None);
    for (index, cell) in cells.iter().enumerate() {
        ingredients[index] = cell.and_then(named);
    }
    Recipe {
        width,
        height,
        shapeless,
        ingredients,
        output: Output {
            id: 9,
            aux: 0,
            count: 1,
            block: 0,
            empty_envelope: false,
        },
    }
}

fn catalog(recipes: Vec<Recipe>) -> RecipeCatalog {
    let owner = Credits::isolated(1 << 20);
    let update = RecipeUpdate {
        batch: Some(Arc::new(Batch {
            records: recipes
                .into_iter()
                .enumerate()
                .map(|(index, recipe)| Record {
                    id: index as u32 + 1,
                    recipe: Some(recipe),
                })
                .collect(),
            clear: true,
            _permit: owner.reserve(1 << 16).unwrap(),
        })),
    };
    let mut catalog = RecipeCatalog::default();
    catalog.begin_session(1);
    assert!(catalog.apply(1, 1, &update));
    catalog
}

fn item(identifier: &str) -> Option<CraftGridItem<'_>> {
    Some(CraftGridItem {
        identifier,
        metadata: 0,
        count: 1,
        plain: true,
    })
}

fn unique_id(result: CraftGridMatch) -> Option<u32> {
    match result {
        CraftGridMatch::Unique(handle) => Some(handle.network_id()),
        _ => None,
    }
}

/// A shaped recipe matches anywhere its exact extent fits, never mirrored.
#[test]
fn shaped_recipes_translate_but_keep_orientation() {
    let catalog = catalog(vec![recipe(2, 1, false, &[Some("a:x"), Some("a:y")])]);
    let mut table = vec![None; 9];
    table[7] = item("a:x");
    table[8] = item("a:y");
    assert_eq!(unique_id(match_crafting_grid(&catalog, 3, &table)), Some(1));
    let mirrored = [item("a:y"), item("a:x"), None, None];
    assert!(matches!(
        match_crafting_grid(&catalog, 2, &mirrored),
        CraftGridMatch::NoMatch
    ));
}

/// A full 3x3 ring needs the table grid and every cell in place.
#[test]
fn three_by_three_recipes_need_the_table_grid() {
    let ring: Vec<Option<&str>> = (0..9)
        .map(|index| (index != 4).then_some("a:stone"))
        .collect();
    let catalog = catalog(vec![recipe(3, 3, false, &ring)]);
    let mut grid: Vec<_> = ring.iter().map(|cell| cell.and_then(item)).collect();
    assert_eq!(unique_id(match_crafting_grid(&catalog, 3, &grid)), Some(1));
    grid[4] = item("a:stone");
    assert!(matches!(
        match_crafting_grid(&catalog, 3, &grid),
        CraftGridMatch::NoMatch
    ));
    assert!(matches!(
        match_crafting_grid(&catalog, 2, &grid[..4]),
        CraftGridMatch::NoMatch
    ));
}

/// Shapeless recipes accept any placement but exactly their ingredients.
#[test]
fn shapeless_recipes_assign_each_cell_once() {
    let catalog = catalog(vec![recipe(0, 0, true, &[Some("a:x"), Some("a:y")])]);
    assert_eq!(
        unique_id(match_crafting_grid(
            &catalog,
            2,
            &[None, item("a:y"), item("a:x"), None]
        )),
        Some(1)
    );
    for grid in [
        [item("a:x"), item("a:x"), None, None],
        [item("a:x"), item("a:y"), item("a:y"), None],
        [item("a:x"), None, None, None],
    ] {
        assert!(matches!(
            match_crafting_grid(&catalog, 2, &grid),
            CraftGridMatch::NoMatch
        ));
    }
}

/// Tags, item data and metadata never match loosely; two recipes are ambiguous.
#[test]
fn tags_data_and_duplicates_fail_closed() {
    let mut tagged = recipe(1, 1, false, &[Some("minecraft:planks")]);
    tagged.ingredients[0].as_mut().unwrap().tag = true;
    let catalog_with_tag = catalog(vec![tagged]);
    assert!(matches!(
        match_crafting_grid(
            &catalog_with_tag,
            2,
            &[item("minecraft:planks"), None, None, None]
        ),
        CraftGridMatch::NoMatch
    ));
    let duplicates = catalog(vec![
        recipe(1, 1, false, &[Some("a:x")]),
        recipe(0, 0, true, &[Some("a:x")]),
    ]);
    assert!(matches!(
        match_crafting_grid(&duplicates, 2, &[item("a:x"), None, None, None]),
        CraftGridMatch::Ambiguous
    ));
    let single = catalog(vec![recipe(1, 1, false, &[Some("a:x")])]);
    let mut data = item("a:x");
    data.as_mut().unwrap().plain = false;
    let mut variant = item("a:x");
    variant.as_mut().unwrap().metadata = 1;
    for cell in [data, variant] {
        assert!(matches!(
            match_crafting_grid(&single, 2, &[cell, None, None, None]),
            CraftGridMatch::NoMatch
        ));
    }
    assert!(matches!(
        match_crafting_grid(
            &RecipeCatalog::default(),
            2,
            &[item("a:x"), None, None, None]
        ),
        CraftGridMatch::Unavailable
    ));
}
