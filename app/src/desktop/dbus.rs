//! Linux desktop calls over the session bus: the xdg-desktop-portal and the notification service.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
};

use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REQUEST: &str = "org.freedesktop.portal.Request";

type Results = HashMap<String, OwnedValue>;

/// Opens `uri` in the desktop's default handler, as `xdg-open` would.
pub(crate) fn open_uri(uri: &str) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let (code, _) = request(
        &connection,
        "org.freedesktop.portal.OpenURI",
        "OpenURI",
        uri,
        HashMap::new(),
    )?;
    match code {
        0 => Ok(()),
        code => Err(zbus::Error::Failure(format!(
            "OpenURI ended with response {code}"
        ))),
    }
}

/// Returns the chosen local file; `None` when the user dismissed the chooser.
pub(crate) fn pick_file(
    title: &str,
    filter_name: &str,
    patterns: &[String],
) -> zbus::Result<Option<PathBuf>> {
    let connection = Connection::session()?;
    let globs = patterns
        .iter()
        .map(|pattern| (0u32, pattern.as_str()))
        .collect::<Vec<_>>();
    let mut options = HashMap::new();
    options.insert("filters", Value::from(vec![(filter_name, globs)]));
    let (code, mut results) = request(
        &connection,
        "org.freedesktop.portal.FileChooser",
        "OpenFile",
        title,
        options,
    )?;
    if code != 0 {
        return Ok(None);
    }
    let uris = results
        .remove("uris")
        .map(Vec::<String>::try_from)
        .transpose()?
        .unwrap_or_default();
    Ok(uris.first().and_then(|uri| file_path(uri)))
}

/// Shows a desktop notification, as `notify-send` would.
pub(crate) fn notify(summary: &str, body: &str) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )?;
    let hints: HashMap<&str, Value> = HashMap::new();
    let _: u32 = proxy.call(
        "Notify",
        &(
            launcher::PRODUCT_NAME,
            0u32,
            "",
            summary,
            body,
            Vec::<&str>::new(),
            hints,
            -1i32,
        ),
    )?;
    Ok(())
}

/// Calls a portal method shaped `(parent_window, argument, options)`; returns its response code
/// (0 success, 1 cancelled, 2 other) and results.
fn request(
    connection: &Connection,
    interface: &str,
    method: &str,
    argument: &str,
    mut options: HashMap<&str, Value<'_>>,
) -> zbus::Result<(u32, Results)> {
    static NEXT_TOKEN: AtomicU32 = AtomicU32::new(0);
    let token = format!(
        "cinnabar_{}_{}",
        std::process::id(),
        NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
    );
    let sender = connection
        .unique_name()
        .ok_or_else(|| zbus::Error::Failure("session bus gave no unique name".into()))?;
    let expected = request_path(sender.as_str(), &token);
    // Subscribing before the call means a fast Response cannot be missed.
    let mut responses =
        Proxy::new(connection, PORTAL, expected.as_str(), REQUEST)?.receive_signal("Response")?;
    options.insert("handle_token", Value::from(token));
    let portal = Proxy::new(connection, PORTAL, PORTAL_PATH, interface)?;
    let handle: OwnedObjectPath = portal.call(method, &("", argument, options))?;
    // Portals older than handle tokens answer on a path of their own choosing.
    if handle.as_str() != expected {
        responses =
            Proxy::new(connection, PORTAL, handle.as_str(), REQUEST)?.receive_signal("Response")?;
    }
    let response = responses
        .next()
        .ok_or_else(|| zbus::Error::Failure("portal request closed without a response".into()))?;
    response.body().deserialize()
}

/// The Request object a portal creates for `token`, per the portal's documented path scheme.
fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("{PORTAL_PATH}/request/{sender}/{token}")
}

fn file_path(uri: &str) -> Option<PathBuf> {
    url::Url::parse(uri).ok()?.to_file_path().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_path_follows_the_sender_and_token() {
        assert_eq!(
            request_path(":1.42", "cinnabar_7_0"),
            "/org/freedesktop/portal/desktop/request/1_42/cinnabar_7_0"
        );
    }

    #[test]
    fn only_local_file_uris_become_paths() {
        assert_eq!(
            file_path("file:///home/dev/My%20Pack.mcpack"),
            Some(PathBuf::from("/home/dev/My Pack.mcpack"))
        );
        assert_eq!(file_path("https://example.test/pack.mcpack"), None);
        assert_eq!(file_path("not a uri"), None);
    }
}
