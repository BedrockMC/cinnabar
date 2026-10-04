//! Host compatibility exports for the launcher installation model.
pub use launcher::install_layout::InstallLayout;
#[cfg(test)]
pub use launcher::install_layout::vanilla_pack_relative;
#[cfg(test)]
pub(crate) use launcher::install_layout::{InstallEnvironment, Platform};

/// Creates an isolated installation tree for app tests that own local files.
#[cfg(test)]
pub(crate) fn scratch(label: &str) -> InstallLayout {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let base = std::env::temp_dir().join(format!(
        "cinnabar-layout-{label}-{}-{nonce}",
        std::process::id()
    ));
    InstallLayout::resolve(
        Platform::Linux,
        &InstallEnvironment {
            executable: base.join("bin/bedrock-client"),
            home: Some(base.clone()),
            local_app_data: None,
            xdg_config_home: Some(base.join("config")),
            xdg_data_home: Some(base.join("data")),
            xdg_runtime_dir: Some(base.join("run")),
        },
    )
    .expect("absolute test roots form a supported installation")
}

#[cfg(test)]
mod tests {
    use crate::asset_startup::{AssetPathSource, select_asset_path_with_default};
    use std::{ffi::OsString, path::PathBuf};
    #[test]
    fn explicit_asset_sources_precede_the_layout_default() {
        let default = PathBuf::from("/bundle/resources/assets/vanilla-v2193.mcbea");
        let environment = select_asset_path_with_default(
            None,
            Some(OsString::from("/override/environment.mcbea")),
            &default,
        );
        assert_eq!(environment.source, AssetPathSource::Environment);
        let command_line = select_asset_path_with_default(
            Some(PathBuf::from("/override/cli.mcbea").as_path()),
            Some(OsString::from("/override/environment.mcbea")),
            &default,
        );
        assert_eq!(command_line.source, AssetPathSource::CommandLine);
        assert_eq!(
            select_asset_path_with_default(None, None, &default).path,
            default
        );
    }
}
