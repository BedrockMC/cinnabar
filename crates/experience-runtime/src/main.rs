use std::path::Path;

use anyhow::{Context, bail};
use experience_runtime::protocol::fixtures;

const USAGE: &str = "usage: experience-runtime write-fixtures <dir>";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, dir] if command == "write-fixtures" => write_fixtures(Path::new(dir)),
        _ => bail!(USAGE),
    }
}

fn write_fixtures(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for (name, json) in fixtures() {
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}
