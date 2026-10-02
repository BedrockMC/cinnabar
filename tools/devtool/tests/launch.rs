use std::process::Command;

/// Reads launch commands without building assets or starting the game.
fn launch(profile: &str, experiences: bool) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("make")
        .current_dir(root)
        .args([
            "-n",
            "-o",
            "assets",
            "-o",
            "physics-assets",
            "-o",
            "audio-pcm-assets",
            "play",
        ])
        .arg(format!("PROFILE={profile}"))
        .arg(format!(
            "CINNABAR_DEV_SERVER_EXPERIENCES={}",
            u8::from(experiences)
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn dev_profile_places_go_binaries_beside_the_debug_client() {
    for (profile, directory) in [("dev", "debug"), ("play", "play"), ("release", "release")] {
        let commands = launch(profile, false);
        for binary in ["bedrock-core", "bedrock-local-server"] {
            assert!(
                commands.contains(&format!("target/{directory}/{binary}")),
                "{commands}"
            );
        }
        assert!(commands.contains(&format!("run --profile {profile}")));
    }
}
