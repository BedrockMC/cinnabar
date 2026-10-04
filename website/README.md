# Cinnabar download page

Build with `python3 website/build.py`. The output in `.local/website/` is static and can be
served behind Forwardme. The app logo, wordmark and color come from the existing original
artwork; download filenames and repository come from `packaging/release-assets.json`.

Windows downloads the release setup executable. Linux shows the one-line installer command;
the script resolves a concrete stable release and verifies its AppImage checksum before
installing into the user's data directory. macOS offers Apple silicon and Intel DMGs because
browser user agents do not reliably distinguish the two architectures.

Publish the contents of the output directory to the site's static document root. No account,
token, backend, or GitHub API request from the visitor's browser is needed. Download links track
the latest stable GitHub release. The install script is also attached to every release.
