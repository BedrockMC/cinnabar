use super::*;

#[tokio::test]
async fn conflicting_start_game_runtime_ids_are_rejected() {
    let transport =
        ScriptTransport::new(CompressionMode::Deflate, SpawnOrder::RadiusThenSpawn, true);
    let error = match LoginSequence::connect_transport(transport, "RustClient").await {
        Ok(_) => panic!("conflicting StartGame packets must fail"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("conflicting StartGame runtime entity ID")
    );
}

#[tokio::test]
async fn unadvertised_optional_resource_pack_stack_does_not_block_login() {
    let transport = ScriptTransport::new_with_pack_stack(
        CompressionMode::Deflate,
        SpawnOrder::RadiusThenSpawn,
        false,
        true,
    );
    let (_, game_data) = LoginSequence::connect_transport(transport, "RustClient")
        .await
        .expect("an unavailable optional pack must not block login");
    assert_eq!(game_data.start_game.runtime_id.actor_runtime_id, RUNTIME_ID);
}
