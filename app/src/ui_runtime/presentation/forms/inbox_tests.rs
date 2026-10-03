//! Real-carrier Inbox rows keep long service text inside their card.

use ui::{DpiScale, UiVisual};

use super::{pack_harness, snapshot};
use crate::menu::{InboxItem, MenuRuntime, MenuScreen};
use crate::ui_runtime::UiRuntime;

#[test]
fn inbox_rows_ellipsize_title_and_summary_without_crossing_the_border() {
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
            let bounds = node.bounds();
            assert!(
                nodes.iter().any(|card| matches!(
                    card.visual(),
                    UiVisual::Solid {
                        color: [30, 30, 31, 255],
                        ..
                    }
                ) && card.bounds().min().x() < bounds.min().x()
                    && card.bounds().min().y() < bounds.min().y()
                    && card.bounds().max().x() > bounds.max().x()
                    && card.bounds().max().y() > bounds.max().y()),
                "text escaped its card: {bounds:?}"
            );
        }
        assert_eq!(rows, 4);
        snapshot::write(&frame, &format!("inbox-bounded-{}", size[0]));
    }
}
