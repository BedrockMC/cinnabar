use std::sync::Arc;

use render::{MAX_UI_DYNAMIC_PAGES, UiTextureCatalog, UiTexturePage, UiTexturePlan};

#[test]
fn mixed_dimensions_plan_native_bytes_before_materialization() {
    let mut dimensions = vec![[1024, 1024]];
    dimensions.extend([[2048, 2048]; 3]);
    dimensions.extend([[256, 256]; 15]);
    let plan = UiTexturePlan::new(&dimensions).unwrap();
    assert_eq!(plan.bytes(), 55 * 1024 * 1024 + 768 * 1024);
    assert_eq!(plan.buckets().len(), 3);
    assert_eq!(plan.locations().len(), dimensions.len());
    assert!(plan.validate_device(2048, 15).is_ok());
    assert!(plan.validate_device(1024, 256).is_err());
    assert!(plan.validate_device(4096, 14).is_err());
}

#[test]
fn planner_checks_entire_catalog_and_all_limits() {
    assert!(UiTexturePlan::new(&[[4096, 4096], [4096, 4096]]).is_ok());
    assert!(UiTexturePlan::new(&[[4096, 4096], [4096, 4096], [1, 1]]).is_err());
    assert!(UiTexturePlan::new(&[[u32::MAX, u32::MAX]]).is_err());
    assert!(UiTexturePlan::new(&[[0, 256]]).is_err());
    assert!(UiTexturePlan::new(&vec![[1, 1]; 257]).is_err());
    let nine_dimensions = (1..=9).map(|n| [n, n]).collect::<Vec<_>>();
    assert!(UiTexturePlan::new(&nine_dimensions).is_err());
    assert!(UiTexturePage::owned([1, 1], vec![0; 3].into()).is_err());
    assert!(UiTexturePage::owned([4097, 1], vec![0; 4].into()).is_err());
    assert!(UiTextureCatalog::new(Vec::new(), 0).is_err());
    let reserved = UiTexturePage::owned([256, 256], vec![0; 256 * 256 * 4].into()).unwrap();
    assert!(UiTextureCatalog::new(vec![reserved; MAX_UI_DYNAMIC_PAGES + 1], 0).is_err());
    let unreserved = UiTexturePage::owned([1, 1], vec![0; 4].into()).unwrap();
    assert!(UiTextureCatalog::new(vec![unreserved], 0).is_err());
}

#[test]
fn dynamic_catalog_reuses_static_pixels_and_retires_old_snapshots() {
    let static_page = UiTexturePage::owned([16, 16], vec![255; 1024].into()).unwrap();
    let initial = UiTexturePage::owned([256, 256], vec![0; 256 * 256 * 4].into()).unwrap();
    let base = UiTextureCatalog::new(vec![static_page.clone(), initial], 1).unwrap();
    let mut current = Arc::new(base.clone());
    let mut prior_pixels = None;
    for value in 1..=100 {
        let retired = Arc::downgrade(&current);
        let pixels: Arc<[u8]> = vec![value; 256 * 256 * 4].into();
        let weak = Arc::downgrade(&pixels);
        let next = UiTexturePage::owned([256, 256], pixels).unwrap();
        current = Arc::new(base.replace_dynamic(vec![next]).unwrap());
        assert!(std::ptr::eq(
            current.pages()[0].pixels(),
            static_page.pixels()
        ));
        assert!(retired.upgrade().is_none());
        if let Some(old) = prior_pixels.take() {
            assert!(std::sync::Weak::upgrade(&old).is_none());
        }
        prior_pixels = Some(weak);
        assert_eq!(current.static_identity(), base.static_identity());
    }
    assert!(base.replace_dynamic(vec![]).is_err());
    assert!(base.replace_dynamic(vec![static_page]).is_err());
}

#[test]
fn ordered_logical_mapping_does_not_group_draw_order() {
    let plan = UiTexturePlan::new(&[[1024, 1024], [2048, 2048], [1024, 1024]]).unwrap();
    let logical_draws = [0usize, 1, 0, 2, 1];
    let physical = logical_draws.map(|logical| plan.locations()[logical]);
    assert_eq!(physical[0].bucket, physical[2].bucket);
    assert_ne!(physical[0].bucket, physical[1].bucket);
    assert_eq!(physical[3].layer, 1);
    assert_eq!(physical[4], physical[1]);
}
