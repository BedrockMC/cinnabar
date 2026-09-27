//! Session-bound form authority and bounded, single-enqueue responses.
mod interaction;
mod network;
mod shape_probe;
use super::UiRuntime;
pub(crate) use interaction::drive_server_form_input;
pub(crate) use network::flush_server_form_network;
use protocol::{
    FormKind, FormRequestEvent, ModalFormResponseSelection, Packet, ServerFormModel,
    modal_form_busy_response, modal_form_cancel_response, modal_form_submit_response,
};
use std::{collections::VecDeque, sync::Arc};

/// One displayed form; at most eight pending busy cancellations.
pub const MAX_RETAINED_SERVER_FORMS: usize = 8;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerFormIdentity {
    pub session: u64,
    pub form_id: u32,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFormEntry {
    pub identity: ServerFormIdentity,
    pub form_id: u32,
    pub kind: FormKind,
    pub title: Option<Arc<str>>,
    pub model: ServerFormModel,
    fifo_sequence: u64,
}
impl ServerFormEntry {
    pub const fn fifo_sequence(&self) -> u64 {
        self.fifo_sequence
    }
    pub fn button_count(&self) -> usize {
        match &self.model {
            ServerFormModel::TextMenu(menu) => menu.buttons.len(),
            _ => 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalFormAction {
    SubmitButton(u32),
    Dismiss,
    CustomElements,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormRespondError {
    StaleIdentity,
    PendingResponse,
    InvalidButton,
    UnsupportedControls,
    CustomElementsUnsupported,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetainedAnswer {
    ButtonIndex(u32),
    Dismissed,
    Busy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingFormResponse {
    identity: ServerFormIdentity,
    answer: RetainedAnswer,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormTransportError {
    /// The queue definitely did not accept the packet: safe to retry.
    Full,
    /// Closed is not a delivery acknowledgement and must not retry.
    Closed,
}
#[derive(Debug, Clone, Default)]
pub struct ServerFormStore {
    active: Option<ServerFormEntry>,
    pending: Option<PendingFormResponse>,
    busy: VecDeque<PendingFormResponse>,
    next_revision: u64,
    replaced_by_reissue: u64,
    dropped_over_capacity: u64,
    watched_dimension: Option<i32>,
    watched_epoch: Option<(u64, u64)>,
    focus: usize,
    scroll: usize,
}
impl ServerFormStore {
    pub fn admit(
        &mut self,
        event: FormRequestEvent,
        fifo_sequence: u64,
        session: u64,
        other_ui: bool,
    ) {
        shape_probe::observe(&event);
        self.next_revision = self.next_revision.saturating_add(1);
        let identity = ServerFormIdentity {
            session,
            form_id: event.form_id,
            revision: self.next_revision,
        };
        // Same-ID reissue is new authority, even while an old answer is Full.
        // Never send an unsent old cancellation against the new revision.
        let replaces = self
            .active
            .as_ref()
            .is_some_and(|entry| entry.form_id == event.form_id)
            || self
                .pending
                .is_some_and(|response| response.identity.form_id == event.form_id);
        self.busy
            .retain(|response| response.identity.form_id != event.form_id);
        if replaces {
            self.active = None;
            self.pending = None;
            self.replaced_by_reissue = self.replaced_by_reissue.saturating_add(1);
        }
        if other_ui || self.active.is_some() || self.pending.is_some() {
            if self.busy.len() < MAX_RETAINED_SERVER_FORMS {
                self.busy.push_back(PendingFormResponse {
                    identity,
                    answer: RetainedAnswer::Busy,
                });
            } else {
                self.dropped_over_capacity = self.dropped_over_capacity.saturating_add(1);
            }
            return;
        }
        self.focus = 0;
        self.scroll = 0;
        self.active = Some(ServerFormEntry {
            identity,
            form_id: event.form_id,
            kind: event.kind,
            title: event.title,
            model: event.model,
            fifo_sequence,
        });
    }
    pub fn active(&self) -> Option<&ServerFormEntry> {
        self.active.as_ref()
    }
    pub fn entries(&self) -> impl Iterator<Item = &ServerFormEntry> {
        self.active.iter()
    }
    pub fn get(&self, form_id: u32) -> Option<&ServerFormEntry> {
        self.active
            .as_ref()
            .filter(|entry| entry.form_id == form_id)
    }
    pub fn owns_input(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }
    pub const fn focus(&self) -> usize {
        self.focus
    }
    pub const fn scroll(&self) -> usize {
        self.scroll
    }
    pub fn move_focus(&mut self, delta: i32) {
        let Some(entry) = self.active.as_ref() else {
            return;
        };
        let count = entry.button_count() + 1;
        self.focus = (self.focus as i64 + i64::from(delta)).rem_euclid(count as i64) as usize;
    }
    pub fn scroll_rows(&mut self, delta: i32) {
        self.scroll = (self.scroll as i64 + i64::from(delta)).clamp(0, 1 << 20) as usize;
    }
    pub fn set_scroll(&mut self, offset: usize) {
        self.scroll = offset.min(1 << 20);
    }
    pub fn reject_active_busy(&mut self) {
        if let Some(entry) = self.active.take() {
            if self.busy.len() < MAX_RETAINED_SERVER_FORMS {
                self.busy.push_back(PendingFormResponse {
                    identity: entry.identity,
                    answer: RetainedAnswer::Busy,
                });
            } else {
                self.dropped_over_capacity = self.dropped_over_capacity.saturating_add(1);
            }
        }
    }
    pub fn respond(
        &mut self,
        identity: ServerFormIdentity,
        action: LocalFormAction,
    ) -> Result<(), FormRespondError> {
        if self.pending.is_some() {
            return Err(FormRespondError::PendingResponse);
        }
        let entry = self
            .active
            .as_ref()
            .filter(|entry| entry.identity == identity)
            .ok_or(FormRespondError::StaleIdentity)?;
        let answer = match action {
            LocalFormAction::CustomElements => {
                return Err(FormRespondError::CustomElementsUnsupported);
            }
            LocalFormAction::Dismiss => RetainedAnswer::Dismissed,
            LocalFormAction::SubmitButton(index) => {
                if !matches!(entry.model, ServerFormModel::TextMenu(_)) {
                    return Err(FormRespondError::UnsupportedControls);
                }
                if index as usize >= entry.button_count() {
                    return Err(FormRespondError::InvalidButton);
                }
                RetainedAnswer::ButtonIndex(index)
            }
        };
        self.pending = Some(PendingFormResponse { identity, answer });
        self.active = None;
        Ok(())
    }
    pub fn note_stream_dimension(&mut self, dimension: i32) {
        if self
            .watched_dimension
            .is_some_and(|previous| previous != dimension)
        {
            self.clear();
        }
        self.watched_dimension = Some(dimension);
    }
    pub fn synchronize_epoch(&mut self, session: u64, dimension_epoch: u64) {
        let identity = (session, dimension_epoch);
        if self.watched_epoch != Some(identity) {
            self.clear();
            self.watched_epoch = Some(identity);
        }
    }
    pub fn clear(&mut self) {
        self.active = None;
        self.pending = None;
        self.busy.clear();
        self.watched_dimension = None;
        self.watched_epoch = None;
        self.focus = 0;
        self.scroll = 0;
    }
    pub const fn replaced_by_reissue(&self) -> u64 {
        self.replaced_by_reissue
    }
    pub const fn dropped_over_capacity(&self) -> u64 {
        self.dropped_over_capacity
    }
    pub fn queued_busy_count(&self) -> usize {
        self.busy.len()
    }
}
fn pending_packet(pending: PendingFormResponse) -> Packet {
    match pending.answer {
        RetainedAnswer::ButtonIndex(index) => modal_form_submit_response(
            pending.identity.form_id,
            ModalFormResponseSelection::ButtonIndex(index),
        ),
        RetainedAnswer::Dismissed => modal_form_cancel_response(pending.identity.form_id),
        RetainedAnswer::Busy => modal_form_busy_response(pending.identity.form_id),
    }
}
/// Drain at most one response. Accepted enqueue is consumed once, never
/// retried on an uncertain subsequent delivery failure.
pub fn flush_form_response(
    runtime: &mut UiRuntime,
    mut send: impl FnMut(Packet) -> Result<(), FormTransportError>,
) -> Result<bool, FormTransportError> {
    let session = runtime.session_id();
    let store = runtime.server_forms_mut();
    let local = store.pending.is_some();
    let Some(pending) = store.pending.take().or_else(|| store.busy.pop_front()) else {
        return Ok(false);
    };
    if pending.identity.session != session {
        return Ok(false);
    }
    match send(pending_packet(pending)) {
        Ok(()) => Ok(true),
        Err(FormTransportError::Full) => {
            if local {
                store.pending = Some(pending);
            } else {
                store.busy.push_front(pending);
            }
            Err(FormTransportError::Full)
        }
        Err(FormTransportError::Closed) => {
            store.clear();
            Err(FormTransportError::Closed)
        }
    }
}
