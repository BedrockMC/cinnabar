//! Production dispatch source witness stays with its app-owned scheduler.

#[test]
fn block_crack_consumer_is_wired_to_the_production_committed_dispatch() {
    let source = include_str!("../runtime/world.rs");
    let drive = source
        .split_once("pub(crate) fn drive_world_stream(")
        .unwrap()
        .1;
    let early = include_str!("../runtime/world/committed_ui.rs");
    let authority = include_str!("../app/authority.rs");
    assert!(early.contains("} => consume_committed_block_crack("));
    assert!(early.contains("stream.take_committed_ui()"));
    assert!(!drive.contains("stream.take_committed_ui()"));
    assert!(authority.contains("drain_committed_ui_before_authority"));
    assert!(authority.contains(".before(ClientFrameSet::UiAuthority)"));
    assert!(drive.contains("reconcile_world_block_cracks(&mut ui_runtime, stream)"));
    assert!(drive.contains("ui_runtime.clear_disconnected_block_cracks()"));
}
