use super::*;

fn decal() -> Decal {
    Decal {
        center: [0.0, 64.0, 0.0],
        radius: 3.0,
        color: [1.0, 0.2, 0.1, 0.8],
        progress: 0.5,
        style: DecalStyle::Telegraph,
    }
}

#[test]
fn appends_are_all_or_nothing_within_budgets() {
    let mut frame = Primitives::default();
    let full = Primitives {
        decals: vec![decal(); mod_api::MAX_RENDER_DECALS],
        ..Default::default()
    };
    frame.append_checked(full).unwrap();
    let one_more = Primitives {
        decals: vec![decal()],
        ..Default::default()
    };
    assert!(frame.append_checked(one_more).is_err());
    assert_eq!(frame.decals.len(), mod_api::MAX_RENDER_DECALS);
}

#[test]
fn non_finite_or_oversized_values_reject_the_whole_append() {
    let bad = [
        Decal {
            radius: f32::NAN,
            ..decal()
        },
        Decal {
            radius: mod_api::MAX_PRIMITIVE_EXTENT_BLOCKS + 1.0,
            ..decal()
        },
        Decal {
            center: [f32::INFINITY, 0.0, 0.0],
            ..decal()
        },
        Decal {
            color: [1.0, 1.0, 1.0, 2.0],
            ..decal()
        },
    ];
    for bad in bad {
        let mut frame = Primitives::default();
        let append = Primitives {
            decals: vec![decal(), bad],
            ..Default::default()
        };
        assert!(frame.append_checked(append).is_err(), "{bad:?}");
        assert!(frame.is_empty());
    }
    let short_ribbon = Primitives {
        ribbons: vec![Ribbon {
            points: vec![[0.0; 3]],
            width: 1.0,
            color: [1.0; 4],
        }],
        ..Default::default()
    };
    assert!(Primitives::default().append_checked(short_ribbon).is_err());
}

#[test]
fn pass_names_are_short_identifiers() {
    assert!(pass_name_valid("boss-aura_2"));
    for name in ["", "Upper", "white space", &"x".repeat(33)] {
        assert!(!pass_name_valid(name), "{name}");
    }
}
