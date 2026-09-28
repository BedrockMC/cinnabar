//! Without the UI carrier, forms and packs stay on the fallback dialog.
use super::super::{UiPresentationRuntime, tests::fixture_font};
use crate::ui_runtime::UiRuntime;
use protocol::{FormKind, FormRequestEvent, ModalDialogForm, ServerFormModel};
use std::sync::Arc;

#[test]
fn modal_without_the_carrier_uses_the_fallback_with_both_buttons() {
    let mut runtime = UiRuntime::new(1);
    let session = runtime.session_id();
    runtime.server_forms_mut().admit(
        FormRequestEvent {
            form_id: 9,
            kind: FormKind::Modal,
            title: None,
            json: Arc::from("{}"),
            model: ServerFormModel::Modal(ModalDialogForm {
                title: Arc::from("Sure?"),
                content: Arc::from("Body"),
                button1: Arc::from("Yes"),
                button2: Arc::from("No"),
            }),
        },
        1,
        session,
        false,
    );
    let mut presentation = UiPresentationRuntime::new(fixture_font()).unwrap();
    presentation.set_server_ui_pack(&[("ui/x.json".to_owned(), b"{}".to_vec())]);
    presentation
        .build(&runtime, 0, [1280, 720], ui::DpiScale::new(1.0).unwrap())
        .unwrap();
    let identity = runtime.server_forms().active().unwrap().identity;
    assert!(presentation.form_engine_frame(identity).is_none());
    assert_eq!(presentation.form_button_count(identity), Some(2));
    assert!(presentation.engine_container_frame().is_none());
}
