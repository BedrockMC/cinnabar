//! Real-carrier Inbox rows keep long service text inside their card.

use ui::{DpiScale, UiVisual};

use super::{pack_harness, snapshot};
use crate::menu::{InboxItem, MenuRuntime, MenuScreen};
use crate::ui_runtime::UiRuntime;

#[test]
fn inbox_rows_ellipsize_titles_and_omit_the_body_summary() {
    let Some(mut presentation) = pack_harness::engine_presentation() else {
        return;
    };
    let mut view = MenuRuntime::new(true, 2, "Test".into()).view();
    view.screen = MenuScreen::Inbox;
    view.feeds.home.inbox = ["First", "Second"]
        .map(|name| InboxItem {
            header: format!("{name} {}", "long title ".repeat(60)),
            body: format!(
                "Summary {}\n{}",
                "wide body ".repeat(80),
                "more text ".repeat(80)
            ),
            category: "News".into(),
            unread: true,
            ..Default::default()
        })
        .into();
    for size in [[1280, 720], [800, 600]] {
        presentation.set_menu_view(Some(view.clone()));
        let frame = presentation
            .build(&UiRuntime::new(1), 0, size, DpiScale::new(1.0).unwrap())
            .unwrap();
        let nodes = pack_harness::menu_nodes(&presentation);
        let mut rows = 0;
        for node in nodes {
            let UiVisual::Text { layout, .. } = node.visual() else {
                continue;
            };
            let text: String = layout
                .glyphs()
                .iter()
                .map(|glyph| glyph.codepoint)
                .collect();
            if !["First", "Second", "Summary"]
                .iter()
                .any(|prefix| text.starts_with(prefix))
            {
                continue;
            }
            rows += 1;
            assert_eq!(layout.line_count(), 1, "{text}");
            assert!(
                text.ends_with('…'),
                "long row must advertise truncation: {text}"
            );
        }
        assert_eq!(rows, 2);
        snapshot::write(&frame, &format!("inbox-bounded-{}", size[0]));
    }
}
