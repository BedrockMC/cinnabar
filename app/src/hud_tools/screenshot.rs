//! F2 frame capture: encodes off-thread and reports the saved file in chat.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};
use crossbeam_channel::{Receiver, Sender};

use crate::ui_runtime::UiRuntime;

type SaveResult = Result<String, String>;

#[derive(Resource)]
struct ScreenshotChannel {
    dir: PathBuf,
    sender: Sender<SaveResult>,
    receiver: Receiver<SaveResult>,
}

pub(super) fn configure(app: &mut App, dir: PathBuf) {
    let (sender, receiver) = crossbeam_channel::unbounded();
    app.insert_resource(ScreenshotChannel {
        dir,
        sender,
        receiver,
    })
    .add_systems(
        Update,
        (capture_on_key, report_saved)
            .chain()
            .before(crate::app::ClientFrameSet::UiPreparation),
    );
    if let Some(path) = std::env::var_os("CINNABAR_CAPTURE_PATH") {
        let frames = std::env::var("CINNABAR_CAPTURE_AFTER_FRAMES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(240);
        app.insert_resource(EnvCapture {
            path: PathBuf::from(path),
            frames,
            done: None,
        })
        .add_systems(Update, capture_from_env);
    }
}

/// Dev capture: `CINNABAR_CAPTURE_PATH` saves the window after
/// `CINNABAR_CAPTURE_AFTER_FRAMES` frames (default 240) and exits.
#[derive(Resource)]
struct EnvCapture {
    path: PathBuf,
    frames: u32,
    done: Option<Receiver<SaveResult>>,
}

fn capture_from_env(
    mut capture: ResMut<EnvCapture>,
    mut commands: Commands,
    mut exits: MessageWriter<AppExit>,
) {
    if let Some(done) = &capture.done {
        if let Ok(result) = done.try_recv() {
            eprintln!("capture: {result:?}");
            exits.write(AppExit::Success);
        }
        return;
    }
    if capture.frames > 0 {
        capture.frames -= 1;
        return;
    }
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let path = capture.path.clone();
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>| {
            let image = captured.image.clone();
            let (path, sender) = (path.clone(), sender.clone());
            std::thread::spawn(move || {
                let _ = sender.send(write_png(image, &path));
            });
        },
    );
    capture.done = Some(receiver);
}

fn capture_on_key(
    menu: Option<Res<crate::menu::MenuRuntime>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    channel: Res<ScreenshotChannel>,
    mut commands: Commands,
) {
    if menu.as_ref().is_some_and(|menu| menu.is_visible())
        || !crate::menu::settings_options::binding_pressed(
            menu.as_deref(),
            "key.screenshot",
            &keys,
            &mouse,
        )
    {
        return;
    }
    let path = unique_path(&channel.dir, SystemTime::now());
    let sender = channel.sender.clone();
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>| {
            let image = captured.image.clone();
            let (path, sender) = (path.clone(), sender.clone());
            std::thread::spawn(move || {
                let _ = sender.send(write_png(image, &path));
            });
        },
    );
}

fn report_saved(
    channel: Res<ScreenshotChannel>,
    mut runtime: ResMut<UiRuntime>,
    time: Res<Time<Real>>,
) {
    let now_millis = u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX);
    for result in channel.receiver.try_iter() {
        let line = match result {
            Ok(name) => format!("Saved screenshot as {name}"),
            Err(error) => format!("Failed to save screenshot: {error}"),
        };
        runtime.push_local_chat_line(Arc::from(line), now_millis);
    }
}

/// Writes the capture as RGB so HDR alpha never reaches the file.
fn write_png(image: Image, path: &Path) -> SaveResult {
    let dynamic = image
        .try_into_dynamic()
        .map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    dynamic
        .to_rgb8()
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned()))
}

/// `yyyy-mm-dd_hh.mm.ss.png` in UTC, suffixed `_1`, `_2`... until unused.
fn unique_path(dir: &Path, now: SystemTime) -> PathBuf {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let stem = utc_stamp(seconds);
    let mut path = dir.join(format!("{stem}.png"));
    let mut suffix = 1;
    while path.exists() {
        path = dir.join(format!("{stem}_{suffix}.png"));
        suffix += 1;
    }
    path
}

fn utc_stamp(epoch_seconds: u64) -> String {
    let days = (epoch_seconds / 86_400) as i64;
    let rem = epoch_seconds % 86_400;
    // Proleptic Gregorian civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}.{:02}.{:02}",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn stamp_matches_known_epochs() {
        assert_eq!(utc_stamp(0), "1970-01-01_00.00.00");
        assert_eq!(utc_stamp(951_782_400 + 86_399), "2000-02-29_23.59.59");
        assert_eq!(utc_stamp(1_785_294_202), "2026-07-29_03.03.22");
    }

    #[test]
    fn colliding_names_gain_a_numeric_suffix() {
        let dir = std::env::temp_dir().join(format!("cinnabar-shot-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let first = unique_path(&dir, now);
        assert!(first.ends_with("2001-09-09_01.46.40.png"));
        fs::write(&first, b"x").unwrap();
        assert!(unique_path(&dir, now).ends_with("2001-09-09_01.46.40_1.png"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
