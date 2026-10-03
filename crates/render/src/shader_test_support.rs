//! Naga checks that tie bind group layouts to what shader stages actually read.

/// Whether the shader's fragment entry point reads the global at `group`/`binding`.
pub(crate) fn fragment_reads_binding(source: &str, group: u32, binding: u32) -> bool {
    let module = naga::front::wgsl::parse_str(source).expect("shader parses");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("shader validates");
    let (index, _) = module
        .entry_points
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.stage == naga::ShaderStage::Fragment)
        .expect("fragment entry point");
    let entry = info.get_entry_point(index);
    module.global_variables.iter().any(|(handle, global)| {
        global
            .binding
            .as_ref()
            .is_some_and(|slot| slot.group == group && slot.binding == binding)
            && !entry[handle].is_empty()
    })
}
