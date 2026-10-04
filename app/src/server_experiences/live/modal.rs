//! The modal screen the client parts draw, and the actions its controls deliver.

use super::{Instance, Live, Worker};
use crate::ui_runtime::presentation::ExperienceModal;
use anyhow::Result;
use mod_host::helper::{Dispatch, Event};
use server_experience::runtime::{Command, Transaction};

/// Presses waiting for one bundle's helper; more before it answers are dropped.
const MAX_PENDING_EVENTS: usize = 16;

/// Raises a bundle's modal above the others when its transaction opened a screen.
pub(super) fn note_opened<H>(
    instance: &mut Instance<H>,
    transaction: &Transaction,
    order: &mut u64,
) {
    if transaction
        .commands
        .iter()
        .any(|command| matches!(command, Command::Screen { template: Some(_) }))
    {
        *order += 1;
        instance.opened = *order;
    }
}

impl<H: Worker> Live<H> {
    /// The modal to draw: the most recently opened screen still open, else the last opened
    /// bundle's closed one, so its catalog and textures stay ready for reopening.
    pub(in crate::server_experiences) fn modal(&self) -> Option<ExperienceModal<'_>> {
        let (bundle, instance) = self
            .instances
            .iter()
            .filter(|(_, instance)| instance.opened > 0)
            .max_by_key(|(_, instance)| {
                (
                    instance.contributions.modal.template.is_some(),
                    instance.opened,
                )
            })?;
        Some(ExperienceModal {
            bundle,
            files: &instance.files,
            modal: &instance.contributions.modal,
        })
    }

    /// Escape closes the open modal on the host side; its bound data stays for the next open.
    pub(in crate::server_experiences) fn close_modal(&mut self) {
        if let Some(instance) = self.open_instance() {
            instance.contributions.modal.open(None);
        }
    }

    /// Queues a press of the open modal's control `id` (the control's `$pressed_button_name`)
    /// for its bundle, only when the manifest declares it as an action and `input` is granted.
    pub(in crate::server_experiences) fn press(&mut self, id: &str, index: Option<usize>) -> bool {
        let ready = self.ready;
        let Some(instance) = self.open_instance() else {
            return false;
        };
        if !ready
            || !instance.capabilities.may_deliver(id)
            || instance.events.len() >= MAX_PENDING_EVENTS
        {
            return false;
        }
        instance.events.push_back(Event::Action {
            id: id.to_owned(),
            index: index.and_then(|index| u32::try_from(index).ok()),
        });
        true
    }

    fn open_instance(&mut self) -> Option<&mut Instance<H>> {
        let bundle = self
            .modal()
            .filter(|modal| modal.modal.template.is_some())?
            .bundle
            .to_owned();
        self.instances.get_mut(&bundle)
    }

    /// Hands each idle helper its oldest waiting event within the aggregate callback budget.
    pub(super) fn deliver_events(&mut self, epoch: u64) -> Result<()> {
        for instance in self.instances.values_mut() {
            if instance.busy
                || instance.events.is_empty()
                || !self.budget.can_dispatch(&instance.owner)
            {
                continue;
            }
            let Some(helper) = &mut instance.helper else {
                continue;
            };
            let event = instance.events.pop_front().expect("events checked");
            self.budget.dispatch(&instance.owner)?;
            instance.callback = event.callback();
            helper.dispatch(Dispatch { event, epoch })?;
            instance.busy = true;
            instance.epoch = epoch;
        }
        Ok(())
    }
}
