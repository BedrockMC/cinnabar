use super::*;

#[test]
fn mining_negotiation_distinguishes_false_from_unknown_and_resets_by_session() {
    let mut runtime = UiRuntime::new(1);
    assert_eq!(runtime.server_authoritative_block_breaking(), None);
    runtime.install_block_breaking_mode(1, true, true);
    assert_eq!(runtime.server_authoritative_block_breaking(), Some(true));
    runtime.install_block_breaking_mode(1, false, true);
    assert_eq!(runtime.server_authoritative_block_breaking(), Some(false));
    runtime.install_block_breaking_mode(0, true, true);
    assert_eq!(runtime.server_authoritative_block_breaking(), Some(false));
    runtime.begin_session(2);
    assert_eq!(runtime.server_authoritative_block_breaking(), None);
    runtime.install_block_breaking_mode(2, true, true);
    runtime.begin_session(2);
    assert_eq!(runtime.server_authoritative_block_breaking(), Some(true));
    // Accepted repeated setup clears explicitly, independently of begin_session.
    runtime.clear_block_breaking_mode();
    runtime.install_block_breaking_mode(2, false, false);
    assert_eq!(runtime.server_authoritative_block_breaking(), None);
}
