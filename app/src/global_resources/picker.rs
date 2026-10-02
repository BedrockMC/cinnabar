//! The import button opens the platform file picker on the import worker.
use std::{path::PathBuf, process::Command};

/// Returns a selected filename, cancellation, or a visible picker error.
pub(super) fn pick() -> Result<Option<PathBuf>, String> {
    let output = if cfg!(target_os = "macos") {
        Command::new("osascript").args(["-e", "POSIX path of (choose file with prompt \"Import resource pack (.mcpack, .mcaddon, .zip)\")"]).output()
    } else if cfg!(target_os = "windows") {
        Command::new("powershell").args(["-NoProfile", "-STA", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Filter = 'Resource packs|*.mcpack;*.mcaddon;*.zip'; if ($d.ShowDialog() -eq 'OK') { [Console]::WriteLine($d.FileName) }"]).output()
    } else {
        Command::new("zenity").args(["--file-selection", "--title=Import resource pack", "--file-filter=Resource packs | *.mcpack *.mcaddon *.zip"]).output()
    }.map_err(|error| format!("File picker unavailable: {error}. Drop a pack onto the window to import it."))?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}
