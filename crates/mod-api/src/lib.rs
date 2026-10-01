//! Guest SDK generated from the same WIT contract used by the host.

pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "extension",
        pub_export_macro: true,
    });
}

/// Versioned SDK for consented server components, separate from personal mods.
pub mod server_bundle {
    wit_bindgen::generate!({
        path: "wit",
        world: "server-bundle",
        pub_export_macro: true,
        export_macro_name: "export_server_bundle",
    });
}
