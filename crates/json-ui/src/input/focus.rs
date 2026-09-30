//! A control's focus component (`UIControlFactory::_populateFocusComponent`).

use crate::tree::ResolvedControl;

/// What focus navigation reads from one control.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusMeta {
    pub enabled: bool,
}

impl FocusMeta {
    /// The focus component `control` carries, `None` for types without one.
    pub(crate) fn read(control: &ResolvedControl) -> Option<Self> {
        let _ = control;
        None
    }
}
