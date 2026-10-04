//! `assetc vanilla-pack`: the development path to the bounded unpack first-run setup uses.

use std::{error::Error, fs, io::Write, path::Path};

use assets::{
    VanillaSource,
    vanilla_pack::{self, UnpackLimits, sha256_file},
};

pub(super) fn acquire(manifest: &Path, accept_eula: bool) -> Result<(), Box<dyn Error>> {
    if !accept_eula {
        return Err(
            "refusing to fetch Mojang assets without the explicit --accept-eula flag".into(),
        );
    }
    let source = VanillaSource::read(manifest)?;
    let paths = source.local_paths(&std::env::current_dir()?)?;
    if paths.is_unpacked() {
        println!(
            "Vanilla source is already available: {}",
            paths.cache.display()
        );
        return Ok(());
    }
    let expected = source.sha256.to_ascii_lowercase();
    if paths.archive.is_file() && sha256_file(&paths.archive)? == expected {
        println!("Using verified archive: {}", paths.archive.display());
    } else {
        if !source.url.starts_with("https://") {
            return Err(format!("sample pack URL is not HTTPS: {}", source.url).into());
        }
        if let Some(parent) = paths.archive.parent() {
            fs::create_dir_all(parent)?;
        }
        println!("Downloading {}", source.url);
        download(&source.url, &paths.partial)?;
        let actual = sha256_file(&paths.partial)?;
        if actual != expected {
            let _ = fs::remove_file(&paths.partial);
            return Err(format!("SHA-256 mismatch: expected {expected}, got {actual}").into());
        }
        fs::rename(&paths.partial, &paths.archive)?;
    }
    vanilla_pack::unpack(&paths, &UnpackLimits::PINNED, &|| false)?;
    println!("Vanilla source ready: {}", paths.cache.display());
    Ok(())
}

fn download(url: &str, partial: &Path) -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut response = reqwest::get(url).await?.error_for_status()?;
        let mut file = fs::File::create(partial)?;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)?;
        }
        file.sync_all()?;
        Ok::<(), Box<dyn Error>>(())
    })
}
