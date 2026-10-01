//! Small SDK tools; none of these commands starts a client or network session.

use anyhow::{Result, bail, ensure};
use mod_host::ModHost;
use std::{hint::black_box, path::Path, time::Instant};

/// Dispatches packaging, an input smoke test, or the bounded callback benchmark.
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, source, output] if command == "pack" => {
            let bytes = std::fs::read(source)?;
            let component = wit_component::ComponentEncoder::default()
                .module(&bytes)?
                .validate(true)
                .encode()?;
            std::fs::write(output, component)?;
        }
        [command, path] if command == "probe" => {
            let mut host = ModHost::load(Path::new(path))?;
            let initial = host.label().map(str::to_owned);
            ensure!(initial.is_some(), "sample did not publish its label");
            host.frame(false)?;
            ensure!(
                host.label() == initial.as_deref(),
                "idle frame changed the label"
            );
            host.frame(true)?;
            ensure!(
                host.label().is_some() && host.label() != initial.as_deref(),
                "sample did not react to input"
            );
            println!("initial={initial:?}\nafter_key={:?}", host.label());
        }
        [command, path] if command == "bench" => benchmark(Path::new(path))?,
        _ => bail!(
            "usage: mod-host pack CORE.wasm COMPONENT.wasm | probe COMPONENT.wasm | bench COMPONENT.wasm"
        ),
    }
    Ok(())
}

/// Measures warmed, idle frame crossings in batches, excluding compilation and UI.
fn benchmark(path: &Path) -> Result<()> {
    let start = Instant::now();
    let mut host = ModHost::load(path)?;
    let load = start.elapsed();
    for _ in 0..10_000 {
        host.frame(black_box(false))?;
    }
    let mut samples = Vec::new();
    for _ in 0..100 {
        let start = Instant::now();
        for _ in 0..1_000 {
            host.frame(black_box(false))?;
            black_box(host.label());
        }
        samples.push(start.elapsed().as_nanos() as f64 / 1_000.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "profile={} arch={} os={} load_ms={:.3} batches=100 frames_per_batch=1000 frame_ns_p50={:.1} frame_ns_p95={:.1} frame_ns_max={:.1}",
        if cfg!(debug_assertions) {
            "dev"
        } else {
            "release"
        },
        std::env::consts::ARCH,
        std::env::consts::OS,
        load.as_secs_f64() * 1000.0,
        samples[50],
        samples[95],
        samples[99]
    );
    Ok(())
}
