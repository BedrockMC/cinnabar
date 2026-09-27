use super::*;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("actor-carrier-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn required_actor_carrier_absence_and_size_failure_name_rebuild_command() {
    let directory = Directory::new();
    let world = directory.0.join("world.mcbea");
    let entity = directory.0.join("entity.mcbeent");
    let error = read_coherent_actor_assets(&world, &entity, [1; 32]).unwrap_err();
    assert!(error.to_string().contains(ACTOR_ASSETS_FILENAME));
    assert!(error.to_string().contains("make actor-assets"));
    let file = std::fs::File::create(actor_asset_path(&world)).unwrap();
    file.set_len(assets::MAX_ACTOR_CARRIER_BYTES as u64 + 1)
        .unwrap();
    let error = read_coherent_actor_assets(&world, &entity, [1; 32]).unwrap_err();
    assert!(error.to_string().contains("exceeds startup byte bound"));
    assert!(error.to_string().contains("make actor-assets"));
}

#[test]
fn startup_rejects_parent_replacement_between_entity_and_actor_load() {
    let directory = Directory::new();
    let world = directory.0.join("world.mcbea");
    let entity = directory.0.join("entity.mcbeent");
    std::fs::write(actor_asset_path(&world), b"invalid carrier").unwrap();
    std::fs::write(&entity, b"replaced parent").unwrap();
    let error =
        read_coherent_actor_assets(&world, &entity, Sha256::digest(b"original parent").into())
            .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("entity carrier changed during startup")
    );
    assert!(error.to_string().contains("make actor-assets"));
}
