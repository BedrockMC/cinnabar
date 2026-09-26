use ui::{TextError, TextLayoutCache};

use super::{TextMetrics, UiPresentationError, bounded_visible_text, text_visual_extent};

pub(super) fn status_extent(
    layouts: &mut TextLayoutCache,
    font: &assets::RuntimeFontCatalog,
    metrics: TextMetrics,
    value: &str,
    width: f32,
) -> Result<Option<f32>, UiPresentationError> {
    match layouts.layout(metrics.request(value, (width * 64.0) as u32, font)) {
        Ok(layout) => Ok(Some(text_visual_extent(&layout, metrics.shadow()))),
        Err(TextError::VisualWidthExceeded { .. } | TextError::WrapLineLimitExceeded { .. }) => {
            Ok(None)
        }
        Err(error) => Err(UiPresentationError::Text(error)),
    }
}

// Launcher status paragraphs alone use whole-word rows. The shared glyph
// layout and gameplay typography remain unchanged. Oversized words are omitted
// as a whole, and a visible ellipsis marks any omitted content.
pub(super) fn status_text(
    layouts: &mut TextLayoutCache,
    font: &assets::RuntimeFontCatalog,
    metrics: TextMetrics,
    value: &str,
    width: f32,
    maximum_extent: f32,
) -> Result<(String, f32), UiPresentationError> {
    let mut rows = Vec::<String>::new();
    let mut row = String::new();
    let mut shortened = false;
    let bounded = bounded_visible_text(value);
    let bounded = if bounded.len() != value.len() {
        bounded
            .rsplit_once(char::is_whitespace)
            .map_or("", |(prefix, _)| prefix)
    } else {
        bounded
    };
    for word in bounded.split_whitespace() {
        let candidate = if row.is_empty() {
            word.to_owned()
        } else {
            format!("{row} {word}")
        };
        let fits_row = |layouts: &mut TextLayoutCache,
                        value: &str|
         -> Result<bool, UiPresentationError> {
            match layouts.layout(metrics.request(value, (width * 64.0) as u32, font)) {
                Ok(layout) => Ok(layout.line_count() == 1),
                Err(
                    TextError::VisualWidthExceeded { .. } | TextError::WrapLineLimitExceeded { .. },
                ) => Ok(false),
                Err(error) => Err(UiPresentationError::Text(error)),
            }
        };
        if fits_row(layouts, &candidate)? {
            row = candidate;
        } else {
            if !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            if rows.len() == 32 || !fits_row(layouts, word)? {
                shortened = true;
                break;
            }
            row = word.to_owned();
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    shortened |= bounded_visible_text(value).len() != value.len();
    loop {
        if shortened {
            let Some(last) = rows.last_mut() else {
                if let Some(extent) = status_extent(layouts, font, metrics, "…", width)? {
                    if extent <= maximum_extent {
                        return Ok(("…".to_owned(), extent));
                    }
                }
                return Ok((String::new(), 0.0));
            };
            loop {
                let candidate = format!("{}…", last.trim_end_matches('…'));
                let layout =
                    layouts.layout(metrics.request(&candidate, (width * 64.0) as u32, font));
                if layout.as_ref().is_ok_and(|layout| layout.line_count() == 1) {
                    *last = candidate;
                    break;
                }
                match layout {
                    Err(TextError::VisualWidthExceeded { .. }) | Ok(_) => {}
                    Err(error) => return Err(UiPresentationError::Text(error)),
                }
                *last = last
                    .rsplit_once(' ')
                    .map_or(String::new(), |(prefix, _)| prefix.to_owned());
                if last.is_empty() {
                    *last = "…".to_owned();
                    break;
                }
            }
        }
        let visible = rows.join("\n");
        if visible.is_empty() {
            return Ok((visible, 0.0));
        }
        if let Some(extent) = status_extent(layouts, font, metrics, &visible, width)? {
            if extent <= maximum_extent {
                return Ok((visible, extent));
            }
        }
        rows.pop();
        shortened = true;
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn status_rows_keep_whole_words_and_bounded_ellipsis() {
        let font = crate::ui_runtime::presentation::tests::fixture_font();
        let mut layouts = TextLayoutCache::new(128, 4 * 1024 * 1024);
        let metrics =
            TextMetrics::for_viewport([1280, 720], ui::DpiScale::new(1.0).unwrap(), Some(2));
        let (visible, extent) = status_text(
            &mut layouts,
            &font,
            metrics,
            "Social: Refresh to try again.",
            216.0,
            40.0,
        )
        .unwrap();
        assert_eq!(visible, "Social: Refresh to\ntry again.");
        assert!(extent <= 40.0);
        let (short, extent) = status_text(
            &mut layouts,
            &font,
            metrics,
            "Social: Refresh to try again.",
            216.0,
            20.0,
        )
        .unwrap();
        assert_eq!(short, "Social: Refresh…");
        assert!(extent <= 20.0);
        let large =
            TextMetrics::for_viewport([1280, 720], ui::DpiScale::new(1.0).unwrap(), Some(3));
        let (short, extent) = status_text(
            &mut layouts,
            &font,
            large,
            "Social: Refresh to try again.",
            297.0,
            30.0,
        )
        .unwrap();
        assert_eq!(short, "Social: Refresh…");
        assert!(extent <= 30.0);
        let (short, extent) = status_text(
            &mut layouts,
            &font,
            metrics,
            &"unbroken".repeat(1000),
            60.0,
            20.0,
        )
        .unwrap();
        assert_eq!(short, "…");
        assert!(extent <= 20.0);
        let (short, extent) =
            status_text(&mut layouts, &font, metrics, "Social: Refresh", 1.0, 20.0).unwrap();
        assert!(short.is_empty());
        assert_eq!(extent, 0.0);
    }
}
