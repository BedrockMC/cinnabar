//! Physical device memory is sampled once on the import worker.

/// Reads physical RAM without using current free memory or process memory pressure.
pub(super) fn physical_bytes() -> u64 {
    let result = read_physical_bytes();
    if result.is_none() {
        bevy::log::warn!(
            "physical memory unavailable; resource packs use the lowest automatic tier"
        );
    }
    result.unwrap_or(0)
}

/// Uses the kernel's byte-valued hardware memory property.
#[cfg(target_os = "macos")]
fn read_physical_bytes() -> Option<u64> {
    let output = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Reads the kernel's total physical RAM count, reported in KiB.
#[cfg(target_os = "linux")]
fn read_physical_bytes() -> Option<u64> {
    let memory = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = memory
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?;
    line.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

/// Queries the operating system's physical-memory property on the worker.
#[cfg(target_os = "windows")]
fn read_physical_bytes() -> Option<u64> {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ])
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Unsupported platforms retain the lowest automatic tier until a native reader exists.
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_physical_bytes() -> Option<u64> {
    None
}
