//! Disk inventory and deletion boundary regressions.

use super::*;

/// Gives each test a private, canonical data directory.
fn fixture() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "cinnabar-storage-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    fs::canonicalize(path).unwrap()
}

#[test]
fn inventory_measures_nested_contents_and_cache_clear_preserves_other_data() {
    let root = fixture();
    let cache = root.join("objects");
    let pack = cache.join("pack");
    fs::create_dir_all(&pack).unwrap();
    fs::write(pack.join("manifest"), [0; 13]).unwrap();
    fs::write(pack.join("texture"), [0; 29]).unwrap();
    fs::write(root.join("account"), b"keep").unwrap();
    let items = entries(&cache).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].bytes, 42);
    remove_entry(&cache, &items[0].path).unwrap();
    assert!(entries(&cache).unwrap().is_empty());
    assert_eq!(fs::read(root.join("account")).unwrap(), b"keep");
    assert!(remove_entry(&cache, &root.join("account")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn linked_roots_and_entries_are_never_deleted_or_counted() {
    let root = fixture();
    let cache = root.join("objects");
    fs::create_dir_all(&cache).unwrap();
    let outside = root.join("keep");
    fs::write(&outside, b"private").unwrap();
    std::os::unix::fs::symlink(&outside, cache.join("escape")).unwrap();
    assert!(entries(&cache).unwrap().is_empty());
    assert!(remove_entry(&cache, &cache.join("escape")).is_err());
    fs::write(cache.join("pack"), [0; 5]).unwrap();
    let linked = root.join("linked");
    std::os::unix::fs::symlink(&cache, &linked).unwrap();
    assert!(remove_entry(&linked, &linked.join("pack")).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"private");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn confirmed_screenshot_delete_only_removes_saved_pngs() {
    let root = fixture();
    let mut layout = InstallLayout::scratch("screenshots");
    layout.user_data_root = root.clone();
    fs::create_dir_all(layout.screenshots_dir()).unwrap();
    fs::create_dir_all(layout.resource_pack_cache_dir()).unwrap();
    let screenshot = layout.screenshots_dir().join("saved.png");
    let other = layout.screenshots_dir().join("notes.txt");
    let pack = layout.resource_pack_cache_dir().join("downloaded-pack");
    fs::write(&screenshot, b"png").unwrap();
    fs::write(&other, b"notes").unwrap();
    fs::write(&pack, b"pack").unwrap();
    let mut menu = MenuRuntime::new(true, 2, "Steve".into());
    menu.layout = layout;
    menu.refresh_storage();
    menu.activate_storage(StorageAction::RequestScreenshots);
    assert!(
        screenshot.exists(),
        "request must not delete before confirmation"
    );
    menu.activate_storage(StorageAction::ConfirmDelete);
    assert!(!screenshot.exists());
    assert!(other.exists());
    assert!(pack.exists());
    fs::remove_dir_all(root).unwrap();
}
