//! Guest SDK generated from the same WIT contract used by the host.

pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "extension",
        pub_export_macro: true,
    });
}
