//! Visual state for edit boxes whose text is owned by the launcher.

use super::ScreenArt;
use crate::menu::{MenuField, MenuView};

#[derive(Clone, Copy)]
pub(in super::super) struct Feedback {
    pub(super) caret: bool,
    pub(super) selected: bool,
}

#[derive(Default)]
pub(in super::super) struct Clock {
    previous: Option<(MenuField, String, bool)>,
    started: f64,
}

impl Clock {
    /// Focus and text changes restart the native caret blink interval.
    pub(in super::super) fn update(&mut self, view: &MenuView, now: f64) -> Option<Feedback> {
        let current = view.field.and_then(|field| {
            let text = match field {
                MenuField::Name => &view.name,
                MenuField::Address => &view.address,
                MenuField::Port => &view.port,
                _ => return None,
            };
            Some((field, text.clone(), view.text_selected))
        });
        if current != self.previous {
            self.started = now;
            self.previous = current;
        }
        self.previous.as_ref().map(|(_, _, selected)| Feedback {
            caret: (((now - self.started).max(0.0) / json_ui::CARET_BLINK_SECONDS) as u64)
                .is_multiple_of(2),
            selected: *selected,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct Target<'a> {
    pub(super) text: &'a str,
    pub(super) placeholder: Option<&'a str>,
    pub(super) feedback: Feedback,
}

impl<'a> Target<'a> {
    /// Resolves authored text/placeholder targets from the actual focused edit component.
    pub(super) fn from_frame(frame: &'a json_ui::FormRender, art: ScreenArt<'_>) -> Option<Self> {
        let feedback = art.edit?;
        let key = art.view?.focused.as_deref()?;
        let edit = frame
            .hits
            .iter()
            .find(|hit| hit.key == key)?
            .widget
            .edit
            .as_ref()?;
        Some(Self {
            text: &edit.text_target.as_ref()?.0,
            placeholder: edit.placeholder.as_deref(),
            feedback,
        })
    }
}
