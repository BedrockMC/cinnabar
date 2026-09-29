//! Per-frame HUD observation and publication.

use super::*;

pub(crate) fn observe_mount_jump_input(
    input: Res<crate::semantic_controls::SemanticInputSnapshot>,
    mut runtime: ResMut<UiRuntime>,
    mut presentation: ResMut<UiPresentationRuntime>,
    time: Res<Time<Real>>,
) {
    let now_millis = u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX);
    runtime.set_mount_jump_held(input.phase(semantic_input::Action::Jump).held, now_millis);
    presentation.hud_frame_mut().tab_list_open =
        input.phase(semantic_input::Action::PlayerList).held;
}

pub(crate) fn platform_safe_area_insets() -> SafeArea {
    SafeArea::ZERO
}

/// Resources beyond Bevy's sixteen-parameter limit.
type PublishExtras<'w> = (
    Res<'w, WorldStreamFramePoll>,
    Res<'w, crate::menu::MenuRuntime>,
    Res<'w, render::HandRigScene>,
    Option<Res<'w, crate::movement::PhysicsCollisionRegistries>>,
    Option<Res<'w, render::RuntimeStageProfiler>>,
);

#[allow(clippy::too_many_arguments)]
pub(crate) fn publish_ui_runtime(
    mut runtime: ResMut<UiRuntime>,
    mut presentation: ResMut<UiPresentationRuntime>,
    mut scene: ResMut<UiRenderScene>,
    stats: Res<UiRenderStats>,
    visibility: Res<CaveVisibilityCache>,
    mut diagnostics_input: ResMut<VisibilityDiagnosticsInput>,
    visibility_diagnostics: Res<VisibilityDiagnostics>,
    render_queue: Res<ChunkRenderQueue>,
    upload_acknowledgements: Res<ChunkUploadAcknowledgements>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut client_world: ResMut<ClientWorld>,
    camera_settings: Res<CameraSettingsAuthority>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    time: Res<Time<Real>>,
    (frame_poll, menu_runtime, hand_rig, collisions, profiler): PublishExtras,
    mut hand: crate::presentation::viewmodel::ViewmodelPublish,
) {
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(render::RuntimeStage::UiPublication));
    let Ok(window) = windows.single() else {
        hand.clear();
        return;
    };
    let physical_size = [window.physical_width(), window.physical_height()];
    if physical_size.contains(&0) {
        hand.clear();
        return;
    }
    let logical_width = physical_size[0] as f32 / window.scale_factor();
    let logical_height = physical_size[1] as f32 / window.scale_factor();
    let Ok(dpi_scale) = DpiScale::new(window.scale_factor()) else {
        hand.clear();
        record_fatal_error(
            &mut client_world.fatal_error,
            "primary window reported an unsupported UI DPI scale".to_owned(),
        );
        return;
    };
    let now_millis = u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX);
    runtime.hud.expire(now_millis);
    if menu_runtime.is_visible() {
        presentation.set_loading_message(None);
        diagnostics_input.set_startup_probe_enabled(false);
    } else {
        let (connected, stream_work_drained) =
            client_world
                .stream
                .as_ref()
                .map_or((false, false), |stream| {
                    let stats = stream.stats();
                    let drained = stats.queued_decode_jobs == 0
                        && stats.in_flight_decode_jobs == 0
                        && stats.pending_light_jobs == 0
                        && stats.in_flight_light_jobs == 0
                        && stats.pending_mesh_jobs == 0
                        && stats.in_flight_mesh_jobs == 0
                        && stats.pending_retry_requests == 0
                        && stats.awaiting_sub_chunk_responses == 0
                        && stats.admitted_world_events == 0
                        && stats.admitted_heavy_events == 0
                        && stream.pending_request_work_count() == 0
                        && stream.outstanding_sub_chunk_count() == 0
                        && stream.pending_mesh_change_count() == 0
                        && stream.unacknowledged_mesh_count() == 0;
                    (true, drained)
                });
        let render_work_drained =
            render_queue.retained_len() == 0 && upload_acknowledgements.is_empty();
        let (startup_released, loading_milestone) = presentation.startup.observe_with_milestone(
            StartupReadinessInput {
                session_generation: runtime.session_id(),
                connected,
                diagnostics_frame_generation: diagnostics_input.frame_generation(),
                snapshot: visibility_diagnostics.snapshot(),
                visible_rendered: visibility.visible_rendered,
                cohort_target_complete: frame_poll
                    .cohort
                    .is_some_and(|status| status.target_is_complete()),
                stream_work_drained,
                render_work_drained,
            },
            now_millis,
        );
        if let Some(milestone) = loading_milestone {
            eprintln!("{milestone}");
        }
        diagnostics_input.set_startup_probe_enabled(presentation.startup.probe_enabled(connected));
        presentation.set_loading_message(if !connected {
            Some("Connecting to server...")
        } else if startup_released {
            None
        } else {
            Some("Loading terrain...")
        });
    }
    runtime.expire_gameplay_effects(now_millis);
    let skin = client_world.stream.as_ref().and_then(|stream| {
        let profile = stream.actor_player_profile(stream.local_player_runtime_id())?;
        let protocol::PlayerSkin::Standard(skin) = &profile.skin else {
            return None;
        };
        render::normalize_actor_skin_cached(&ActorSkinPixels {
            width: skin.width,
            height: skin.height,
            rgba8: Arc::clone(&skin.rgba8),
        })
    });
    let pose = client_world
        .stream
        .as_ref()
        .and_then(|stream| stream.actor(stream.local_player_runtime_id()))
        .map_or_else(player_preview::PlayerPreviewPose::default, |actor| {
            let sneaking = matches!(
                actor.metadata.get(&0),
                Some(protocol::ActorMetadataValue::Flags(flags)) if flags & (1_u64 << 1) != 0
            );
            player_preview::PlayerPreviewPose::new(
                actor.body_yaw,
                actor.head_yaw,
                actor.pitch,
                sneaking,
            )
        });
    // The paper doll shows in the inventory and menus; the CPU hands only while no GPU hand rig.
    let first_person =
        camera_settings.perspective() == semantic_input::PerspectiveMode::FirstPerson;
    presentation.sync_player_preview(
        skin.as_deref(),
        pose,
        runtime.inventory_open() || menu_runtime.is_visible(),
        first_person && !hand_rig.is_active(),
    );
    refresh_hud_frame(
        &mut runtime,
        &mut presentation,
        client_world.stream.as_ref(),
        &camera_settings,
        now_millis,
    );
    // When the local player's first-person rig is drawing near-camera, it owns the hand; the
    // static empty-hand scene and the HUD's CPU hand/item carriers are retired so nothing
    // double-draws.
    presentation.hud_frame.hand_rig_active = hand_rig.is_active();
    if hand_rig.is_active() {
        hand.use_animated_rig();
    } else {
        hand.observe(
            &runtime,
            &client_world,
            presentation.hud_frame.first_person,
            menu_runtime.is_visible() || presentation.loading_message.is_some(),
            physical_size,
        );
    }
    let anchors = client_world
        .stream
        .as_ref()
        .zip(cameras.single().ok())
        .map(|(stream, (camera, transform))| {
            project_below_name_anchors(
                runtime.scoreboards(),
                stream,
                camera,
                transform,
                [logical_width, logical_height],
                presentation.safe_area,
            )
        })
        .unwrap_or_default();
    presentation.set_below_name_anchors(anchors);
    let nametags = client_world
        .stream
        .as_ref()
        .zip(cameras.single().ok())
        .map(|(stream, (camera, transform))| {
            project_nametags(
                runtime.scoreboards(),
                stream,
                camera,
                transform,
                [logical_width, logical_height],
                presentation.safe_area,
                collisions.as_deref(),
            )
        })
        .unwrap_or_default();
    presentation.set_nametag_anchors(nametags);
    let menu_view = menu_runtime.is_visible().then(|| {
        let mut view = menu_runtime.view();
        presentation.sync_menu_artwork(super::menu_artwork::view_paths(&view));
        for server in view.featured.iter_mut().chain(view.gatherings.iter_mut()) {
            server.icon = presentation.menu_artwork_icon(&server.image_path);
        }
        view.featured_icon = presentation.item_icon("minecraft:compass_item", 0);
        view.gathering_icon = presentation.item_icon("minecraft:map_empty", 0);
        view.realm_icon = presentation.item_icon("minecraft:ender_pearl", 0);
        view.friend_icon = presentation.item_icon("minecraft:heart_of_the_sea", 0);
        view.saved_icon = presentation.item_icon("minecraft:book_normal", 0);
        view.profile_icon = presentation.player_preview_icon();
        view
    });
    presentation.set_menu_view(menu_view);
    presentation
        .refresh_scoreboard_owner_names(runtime.scoreboards(), client_world.stream.as_ref());
    let input = match presentation.build(&runtime, now_millis, physical_size, dpi_scale) {
        Ok(input) => input,
        Err(error) => {
            hand.clear();
            record_fatal_error(&mut client_world.fatal_error, error.to_string());
            return;
        }
    };
    if !hand_rig.is_active() {
        hand.bind_cpu_fallback(
            &input,
            presentation.cpu_empty_hand_fallback(),
            presentation.hud_frame.held_item_icon,
        );
    }
    if let Err(error) = scene.publish(input, &stats) {
        hand.clear();
        record_fatal_error(
            &mut client_world.fatal_error,
            UiPresentationError::Render(error).to_string(),
        );
    }
}

pub(crate) fn refresh_hud_frame(
    runtime: &mut UiRuntime,
    presentation: &mut UiPresentationRuntime,
    stream: Option<&client_world::WorldStream>,
    camera_settings: &CameraSettingsAuthority,
    now_millis: u64,
) {
    let resolve_identifier = |stack: &protocol::NetworkItemStack| {
        stream.and_then(|stream| stream.canonical_item_stack(stack)?.identifier)
    };
    let derived_armor = runtime.gameplay_hud().armor().map(|slots| {
        let identifiers = [
            &slots.helmet,
            &slots.chestplate,
            &slots.leggings,
            &slots.boots,
        ]
        .map(|stack| {
            (!stack.is_empty())
                .then(|| resolve_identifier(stack))
                .flatten()
        });
        item_facts::total_armor_points(identifiers.iter().map(|id| id.as_deref()))
    });
    runtime.set_derived_armor(derived_armor);
    let mount_health = runtime
        .gameplay_hud()
        .mount_unique_id()
        .and_then(|unique| stream.and_then(|stream| stream.actor_health_by_unique(unique)));
    let mut hotbar_durability = [None; 9];
    let mut hotbar_icons = [None; 9];
    let mut hotbar_stacks: [Option<protocol::NetworkItemStack>; 9] = Default::default();
    let mut logged_hotbar: [Option<(Arc<str>, bool)>; 9] = Default::default();
    let mut inventory_icons = super::hud_layout::InventoryIcons::default();
    for (slot, icon) in inventory_icons.0.iter_mut().enumerate() {
        if let Some(stack) = runtime.inventory_ledger().displayed_stack(slot as u8) {
            *icon = resolve_identifier(stack)
                .as_deref()
                .and_then(|id| presentation.item_icon(id, stack.metadata));
        }
    }
    let mut storage_icons = super::hud_layout::StorageIcons::default();
    for (slot, icon) in storage_icons.0.iter_mut().enumerate() {
        if let Some(stack) = runtime.inventory_ledger().storage_stack(slot as u8) {
            *icon = resolve_identifier(stack)
                .as_deref()
                .and_then(|id| presentation.item_icon(id, stack.metadata));
        }
    }
    let mut crafting = super::hud_layout::CraftingFrame::default();
    if runtime.inventory_open() {
        let ledger = runtime.inventory_ledger();
        for (icon, slot) in crafting
            .icons
            .iter_mut()
            .zip(ledger.crafting_grid().slots())
        {
            let target = crate::ui_runtime::inventory_ledger::InventoryTarget::Craft(slot);
            if let Some(stack) = ledger.target_stack(target) {
                *icon = resolve_identifier(stack)
                    .as_deref()
                    .and_then(|id| presentation.item_icon(id, stack.metadata));
            }
        }
        if let protocol::CraftGridMatch::Unique(recipe) = runtime.crafting_match() {
            let output = recipe.output();
            let stack = protocol::NetworkItemStack {
                network_id: output.network_id,
                metadata: u32::from(output.aux),
                count: u16::from(output.count),
                block_runtime_id: output.block_runtime_id as i32,
                ..protocol::NetworkItemStack::empty()
            };
            let icon = resolve_identifier(&stack)
                .as_deref()
                .and_then(|id| presentation.item_icon(id, stack.metadata));
            crafting.output = Some((icon, stack));
        }
    }
    // Hover names for the open container's cells (JSON-UI tooltips).
    let item_names = if runtime.inventory_open() {
        let ledger = runtime.inventory_ledger();
        (0..36u8)
            .filter_map(|slot| ledger.displayed_stack(slot))
            .chain((0..54u8).filter_map(|slot| ledger.storage_stack(slot)))
            .filter_map(|stack| {
                let name = runtime.localized_item_name(&resolve_identifier(stack)?);
                Some(((stack.network_id, stack.metadata), Arc::from(name)))
            })
            .collect()
    } else {
        Default::default()
    };
    let mut durability = super::hud_layout::Durability::default();
    let mut window_icons = super::hud_layout::WindowIcons::default();
    if runtime.inventory_open() {
        for (row, name) in ["helmet", "chestplate", "leggings", "boots"]
            .iter()
            .enumerate()
        {
            window_icons.ghost_armor[row] =
                presentation.item_icon(&format!("minecraft:empty_armor_slot_{name}"), 0);
        }
        window_icons.ghost_shield = presentation.item_icon("minecraft:empty_armor_slot_shield", 0);
        window_icons.ghost_template =
            presentation.item_icon("minecraft:empty_slot_smithing_template", 0);
    }
    {
        use crate::ui_runtime::inventory_ledger::InventoryTarget;
        let ledger = runtime.inventory_ledger();
        let fraction = |stack: &protocol::NetworkItemStack, correction: Option<i32>| {
            let identifier = resolve_identifier(stack);
            item_facts::cell_durability_fraction(stack, identifier.as_deref(), correction)
        };
        for (slot, bar) in durability.player.iter_mut().enumerate() {
            if let Some(stack) = ledger.displayed_stack(slot as u8) {
                let correction = ledger
                    .presented_slot_overlay(slot as u8)
                    .and_then(|overlay| overlay.durability_correction);
                *bar = fraction(stack, correction);
            }
        }
        for (slot, bar) in durability.storage.iter_mut().enumerate() {
            if let Some(stack) = ledger.storage_stack(slot as u8) {
                *bar = fraction(stack, None);
            }
        }
        for slot in 0..protocol::UI_SLOT_COUNT as u8 {
            let stack = if slot == protocol::CREATED_OUTPUT_SLOT {
                ledger.created_output_stack()
            } else if protocol::ui_slot_container_name(slot).is_some() {
                ledger.target_stack(InventoryTarget::Craft(slot))
            } else {
                None
            };
            if let Some(stack) = stack {
                durability.ui[usize::from(slot)] = fraction(stack, None);
                window_icons.ui[usize::from(slot)] = resolve_identifier(stack)
                    .as_deref()
                    .and_then(|id| presentation.item_icon(id, stack.metadata));
            }
        }
    }
    if runtime.inventory_open()
        && runtime.inventory_ledger().window_kind() == Some(protocol::WindowKind::Lectern)
        && runtime.screen_state().book.is_none()
        && let Some(position) = runtime.inventory_ledger().window_position()
        && let Some(nbt) = stream.and_then(|stream| stream.block_entity_compound(position))
    {
        let pages: Vec<String> = nbt
            .compound("book")
            .and_then(|book| book.compound("tag"))
            .and_then(|tag| tag.list("pages"))
            .map(|pages| {
                pages
                    .iter()
                    .map(|page| match page {
                        world::NbtValue::Compound(page) => {
                            page.string("text").unwrap_or_default().to_owned()
                        }
                        _ => String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut state = crate::ui_runtime::book_screen::BookState::new(
            crate::ui_runtime::book_screen::BookSource::Lectern(position),
            pages,
            false,
            String::new(),
            String::new(),
        );
        state.page = nbt
            .integer("page")
            .and_then(|page| usize::try_from(page).ok())
            .map_or(0, |page| page.min(state.pages.len() - 1));
        runtime.open_book(state);
    }
    let inventory_screen = super::inventory_pointer::InventoryScreen::of_runtime(runtime);
    let mut window_text = super::hud_layout::WindowText::default();
    if runtime.inventory_open() {
        let stated_title = runtime
            .inventory_ledger()
            .window_position()
            .and_then(|position| stream?.block_entity_custom_name(position));
        window_text.inventory_label = runtime
            .translation("container.inventory")
            .map(|text| text.to_string());
        window_text.title = match inventory_screen {
            super::inventory_pointer::InventoryScreen::Window(kind, _) => {
                stated_title.or_else(|| {
                    runtime
                        .translation(super::hud_layout::title_key(kind))
                        .map(|text| text.to_string())
                })
            }
            super::inventory_pointer::InventoryScreen::Storage(count) => {
                stated_title.or_else(|| {
                    runtime
                        .translation(if count == 54 {
                            "container.chestDouble"
                        } else {
                            "container.chest"
                        })
                        .map(|text| text.to_string())
                })
            }
            super::inventory_pointer::InventoryScreen::Workbench => stated_title.or_else(|| {
                runtime
                    .translation("container.crafting")
                    .map(|text| text.to_string())
            }),
            super::inventory_pointer::InventoryScreen::Creative => {
                let key = match runtime.screen_state().creative_tab {
                    0 => "itemGroup.name.construction",
                    1 => "itemGroup.name.nature",
                    2 => "itemGroup.name.equipment",
                    3 => "itemGroup.name.items",
                    _ => "itemGroup.name.search",
                };
                runtime.translation(key).map(|text| text.to_string())
            }
            super::inventory_pointer::InventoryScreen::Personal
            | super::inventory_pointer::InventoryScreen::Book => None,
        };
        if matches!(
            inventory_screen,
            super::inventory_pointer::InventoryScreen::Window(protocol::WindowKind::Beacon, _)
        ) {
            window_text.effect_names = [
                (1, "effect.moveSpeed"),
                (3, "effect.digSpeed"),
                (11, "effect.resistance"),
                (8, "effect.jump"),
                (5, "effect.damageBoost"),
                (10, "effect.regeneration"),
            ]
            .iter()
            .map(|(id, key)| {
                let name = runtime
                    .translation(key)
                    .map_or_else(|| (*key).to_owned(), |text| text.to_string());
                (*id, name)
            })
            .collect();
        }
        if inventory_screen == super::inventory_pointer::InventoryScreen::Creative {
            let entries = crate::ui_runtime::inventory_actions::visible_creative_entries(
                runtime.inventory_ledger(),
                runtime.screen_state(),
            );
            let first = runtime.screen_state().creative_row * super::screens::GRID_COLUMNS;
            for (cell, item) in entries
                .iter()
                .skip(first)
                .take(super::screens::GRID_CELLS)
                .enumerate()
            {
                window_icons.creative[cell] = resolve_identifier(&item.stack)
                    .as_deref()
                    .and_then(|id| presentation.item_icon(id, item.stack.metadata));
            }
            for (tab, id) in [
                "minecraft:brick",
                "minecraft:oak_sapling",
                "minecraft:iron_sword",
                "minecraft:stick",
                "minecraft:compass",
            ]
            .iter()
            .enumerate()
            {
                window_icons.creative_tabs[tab] = presentation.item_icon(id, 0);
            }
        }
        if runtime.inventory_ledger().window_kind() == Some(protocol::WindowKind::Beacon) {
            let level = runtime
                .inventory_ledger()
                .window_position()
                .and_then(|position| stream?.block_entity_compound(position))
                .and_then(|nbt| nbt.integer("Levels"))
                .and_then(|levels| u8::try_from(levels).ok());
            runtime.screen_state_mut().beacon_level = level;
        }
        if let Some(kind) = runtime.inventory_ledger().window_kind() {
            let output_stack = |output: protocol::RecipeOutput| protocol::NetworkItemStack {
                network_id: output.network_id,
                metadata: u32::from(output.aux),
                count: u16::from(output.count),
                block_runtime_id: i32::try_from(output.block_runtime_id).unwrap_or(0),
                ..protocol::NetworkItemStack::empty()
            };
            if kind == protocol::WindowKind::Stonecutter {
                let outputs: Vec<_> = runtime
                    .stonecutter_options()
                    .iter()
                    .take(super::screens::STONECUTTER_CELLS)
                    .map(|recipe| recipe.output)
                    .collect();
                for (cell, output) in outputs.into_iter().enumerate() {
                    if let Some(output) = output {
                        let stack = output_stack(output);
                        window_icons.recipe[cell] = resolve_identifier(&stack)
                            .as_deref()
                            .and_then(|id| presentation.item_icon(id, stack.metadata));
                    }
                }
            }
            if matches!(
                kind,
                protocol::WindowKind::Stonecutter
                    | protocol::WindowKind::Smithing
                    | protocol::WindowKind::Cartography
            ) && runtime.inventory_ledger().created_output_stack().is_none()
                && let Some(output) = runtime
                    .active_screen_recipe()
                    .and_then(|recipe| recipe.output)
            {
                let stack = output_stack(output);
                let icon = resolve_identifier(&stack)
                    .as_deref()
                    .and_then(|id| presentation.item_icon(id, stack.metadata));
                window_icons.recipe_output = Some((icon, stack));
            }
        }
        if matches!(
            inventory_screen,
            super::inventory_pointer::InventoryScreen::Personal
                | super::inventory_pointer::InventoryScreen::Workbench
        ) {
            window_icons.book_button = presentation.item_icon("minecraft:book", 0);
            window_text.book_title = runtime
                .translation("recipe.book.title")
                .map(|text| text.to_string());
            if runtime.screen_state().book_open {
                let first = runtime.screen_state().book_page * super::screens::BOOK_CELLS;
                let page = runtime.book_recipes(first, super::screens::BOOK_CELLS + 1);
                window_icons.book_more = page.len() > super::screens::BOOK_CELLS;
                for (cell, recipe) in page.iter().take(super::screens::BOOK_CELLS).enumerate() {
                    let output = recipe.output();
                    let stack = protocol::NetworkItemStack {
                        network_id: output.network_id,
                        metadata: u32::from(output.aux),
                        count: u16::from(output.count),
                        block_runtime_id: i32::try_from(output.block_runtime_id).unwrap_or(0),
                        ..protocol::NetworkItemStack::empty()
                    };
                    window_icons.book[cell] = resolve_identifier(&stack)
                        .as_deref()
                        .and_then(|id| presentation.item_icon(id, stack.metadata));
                }
            }
        }
        // The tooltip follows the hovered cell's stack.
        let hovered = runtime.screen_state().hover.and_then(|hit| {
            use super::inventory_pointer::InventoryCellHit as Hit;
            let ledger = runtime.inventory_ledger();
            let (stack, name) = match hit {
                Hit::Player(slot) => (
                    ledger.displayed_stack(slot),
                    ledger
                        .presented_slot_overlay(slot)
                        .and_then(|overlay| overlay.custom_name.clone()),
                ),
                Hit::Storage(slot) => (ledger.storage_stack(slot), None),
                Hit::Craft(slot) => (
                    ledger.target_stack(
                        crate::ui_runtime::inventory_ledger::InventoryTarget::Craft(slot),
                    ),
                    None,
                ),
                Hit::Armor(row) => (
                    ledger.target_stack(
                        crate::ui_runtime::inventory_ledger::InventoryTarget::Armor(row),
                    ),
                    None,
                ),
                Hit::Offhand => (
                    ledger.target_stack(
                        crate::ui_runtime::inventory_ledger::InventoryTarget::Offhand,
                    ),
                    None,
                ),
                Hit::CraftOutput => (ledger.created_output_stack(), None),
                Hit::CreativeGrid(index) => {
                    let position = runtime.screen_state().creative_row
                        * super::screens::GRID_COLUMNS
                        + usize::from(index);
                    let entries = crate::ui_runtime::inventory_actions::visible_creative_entries(
                        ledger,
                        runtime.screen_state(),
                    );
                    (entries.get(position).copied().map(|item| &item.stack), None)
                }
                Hit::Widget(super::screens::Widget::BookRecipe(index)) => {
                    let skip = runtime.screen_state().book_page * super::screens::BOOK_CELLS
                        + usize::from(index);
                    let output = runtime
                        .book_recipes(skip, 1)
                        .first()
                        .map(protocol::RecipeHandle::output);
                    return output.map(|output| {
                        let stack = protocol::NetworkItemStack {
                            network_id: output.network_id,
                            metadata: u32::from(output.aux),
                            count: u16::from(output.count),
                            block_runtime_id: i32::try_from(output.block_runtime_id).unwrap_or(0),
                            ..protocol::NetworkItemStack::empty()
                        };
                        (stack, None)
                    });
                }
                Hit::Widget(_) | Hit::CreativeTab(_) | Hit::CreativeSearch => (None, None),
            };
            stack.map(|stack| (stack.clone(), name))
        });
        if let Some((stack, name)) = hovered {
            let identifier = resolve_identifier(&stack);
            window_text.tooltip = super::inventory_tooltip::tooltip_lines(
                runtime,
                &stack,
                identifier.as_deref(),
                name.as_deref(),
            );
            if let Some(contents) = protocol::item_bundle_id(&stack.extra_data)
                .and_then(|id| runtime.inventory_ledger().bundle_contents(id))
            {
                for held in contents.iter().filter(|held| !held.is_empty()).take(8) {
                    let item_name = resolve_identifier(held)
                        .map_or_else(|| "?".to_owned(), |id| runtime.localized_item_name(&id));
                    window_text.tooltip.push(super::hud_layout::TooltipLine {
                        text: format!("{}x {item_name}", held.count),
                        color: [200, 200, 200, 255],
                    });
                }
            }
        }
    }
    let cursor_icon = runtime.inventory_ledger().cursor_stack().and_then(|stack| {
        resolve_identifier(stack)
            .as_deref()
            .and_then(|id| presentation.item_icon(id, stack.metadata))
    });
    let selected_snapshot = runtime.selected_stack_snapshot();
    let selected_slot = selected_snapshot.map(|snapshot| snapshot.slot);
    let selected_stack = selected_snapshot.and_then(|snapshot| match snapshot.state {
        crate::ui_runtime::inventory_ledger::PlayerInventorySlot::Present(stack) => Some(stack),
        crate::ui_runtime::inventory_ledger::PlayerInventorySlot::Unknown
        | crate::ui_runtime::inventory_ledger::PlayerInventorySlot::Empty => None,
    });
    for (slot, durability) in hotbar_durability.iter_mut().enumerate() {
        let slot = slot as u8;
        let stack = if selected_slot == Some(slot) {
            selected_stack
        } else {
            runtime.inventory_ledger().displayed_stack(slot)
        };
        if let Some(stack) = stack {
            let identifier = resolve_identifier(stack);
            // Every hotbar cell — selected or not — derives from one
            // ledger-snapshot authority revision: the travelling predicted
            // overlay beside its predicted stack while a gesture is in
            // flight, otherwise the committed overlay. One accepted sparse
            // response therefore refreshes the whole presented row at once.
            let overlay = runtime.inventory_ledger().presented_slot_overlay(slot);
            *durability = item_facts::cell_durability_fraction(
                stack,
                identifier.as_deref(),
                overlay.and_then(|overlay| overlay.durability_correction),
            );
            hotbar_icons[usize::from(slot)] = identifier
                .as_deref()
                .and_then(|id| presentation.item_icon(id, stack.metadata));
            hotbar_stacks[usize::from(slot)] = Some(stack.clone());
            logged_hotbar[usize::from(slot)] = Some((
                identifier
                    .unwrap_or_else(|| Arc::from(format!("<network id {}>", stack.network_id))),
                hotbar_icons[usize::from(slot)].is_some(),
            ));
        }
    }
    presentation.note_hotbar(logged_hotbar);
    let offhand_durability = runtime.gameplay_hud().offhand_stack().and_then(|stack| {
        let identifier = resolve_identifier(stack);
        item_facts::durability_fraction(stack, identifier.as_deref())
    });
    let offhand_icon = runtime.gameplay_hud().offhand_stack().and_then(|stack| {
        let identifier = resolve_identifier(stack);
        identifier
            .as_deref()
            .and_then(|id| presentation.item_icon(id, stack.metadata))
    });
    let armor_icons = runtime.gameplay_hud().armor().map_or([None; 4], |armor| {
        [
            &armor.helmet,
            &armor.chestplate,
            &armor.leggings,
            &armor.boots,
        ]
        .map(|stack| {
            resolve_identifier(stack)
                .as_deref()
                .and_then(|id| presentation.item_icon(id, stack.metadata))
        })
    });
    let held_item_icon = selected_stack.and_then(|stack| {
        resolve_identifier(stack)
            .as_deref()
            .and_then(|id| presentation.item_icon(id, stack.metadata))
    });
    presentation.set_item_viewmodels(held_item_icon, offhand_icon);
    let (held_viewmodel_icon, offhand_viewmodel_icon) = presentation.item_viewmodel_icons();
    let selected_item_name = runtime.selected_stack_custom_name().or_else(|| {
        selected_stack.and_then(|stack| {
            resolve_identifier(stack).map(|id| Arc::from(runtime.localized_item_name(&id)))
        })
    });
    let selected_identity = selected_stack.map(|stack| (stack.network_id, stack.metadata));
    let mount_jump = runtime.gameplay_hud().mount_unique_id().and_then(|unique| {
        stream
            .filter(|stream| {
                stream.actor_has_attribute_by_unique(unique, "minecraft:horse.jump_strength")
            })
            .map(|_| runtime.mount_jump_charge(now_millis))
    });
    let first_person =
        camera_settings.perspective() == semantic_input::PerspectiveMode::FirstPerson;
    let player_preview_icon = presentation.player_preview_icon();
    let (left_hand_icon, right_hand_icon) = presentation.player_hand_icons();
    runtime.observe_selected_item_identity_value(selected_identity, now_millis);
    let sleeping = stream
        .and_then(|stream| stream.actor(stream.local_player_runtime_id()))
        .is_some_and(|actor| actor.is_sleeping());
    runtime.set_local_sleeping(sleeping);
    let frame = presentation.hud_frame_mut();
    frame.sleep.observe(sleeping, now_millis);
    frame.first_person = first_person;
    frame.mount_health = mount_health;
    frame.hotbar_durability = hotbar_durability;
    frame.hotbar_stacks = hotbar_stacks;
    frame.offhand_durability = offhand_durability;
    frame.hotbar_icons = hotbar_icons;
    frame.inventory_icons = inventory_icons;
    frame.storage_icons = storage_icons;
    frame.crafting = crafting;
    frame.window_icons = window_icons;
    frame.durability = durability;
    frame.window_text = window_text;
    frame.cursor_icon = cursor_icon;
    frame.armor_icons = armor_icons;
    frame.offhand_icon = offhand_icon;
    frame.offhand_viewmodel_icon = offhand_viewmodel_icon;
    frame.held_item_icon = held_viewmodel_icon;
    frame.player_preview = player_preview_icon;
    frame.left_hand = left_hand_icon;
    frame.right_hand = right_hand_icon;
    frame.viewmodel_pitch_degrees = stream
        .and_then(|stream| stream.actor(stream.local_player_runtime_id()))
        .map_or(0.0, |actor| actor.pitch);
    frame.selected_item_name = selected_item_name;
    frame.item_names = item_names;
    frame.mount_jump = mount_jump;
    frame.attack_indicator_charge = Some(1.0);
    let diagnostics = runtime.gameplay_hud().diagnostics();
    if diagnostics != presentation.last_hud_diagnostics {
        bevy::log::debug!(
            skipped_effect_actions = diagnostics.skipped_effect_actions,
            evicted_effects = diagnostics.evicted_effects,
            odd_metadata_values = diagnostics.odd_metadata_values,
            dropped_inventory_events = diagnostics.dropped_inventory_events,
            unknown_container_events = diagnostics.unknown_container_events,
            odd_attribute_values = diagnostics.odd_attribute_values,
            odd_hud_packets = diagnostics.odd_hud_packets,
            oversized_chat_rows = diagnostics.oversized_chat_rows,
            unknown_effect_ids = diagnostics.unknown_effect_ids,
            "gameplay HUD skipped odd remote data"
        );
        presentation.last_hud_diagnostics = diagnostics;
    }
}

fn project_below_name_anchors(
    scoreboards: &ui::ScoreboardStore,
    stream: &client_world::WorldStream,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    logical_size: [f32; 2],
    safe_area: SafeArea,
) -> Vec<BelowNameAnchor> {
    let content_width = (logical_size[0] - safe_area.left() - safe_area.right()).max(0.0);
    let content_height = (logical_size[1] - safe_area.top() - safe_area.bottom()).max(0.0);
    stream
        .render_players()
        .into_iter()
        .filter_map(|(actor, _profile)| {
            let below_name = scoreboards
                .below_name_for_owner(&ui::ScoreOwner::Player(actor.unique_id))
                .or_else(|| {
                    scoreboards.below_name_for_owner(&ui::ScoreOwner::Entity(actor.unique_id))
                })?;
            let name = stream.actor_display_name(actor.unique_id)?;
            let position = Vec3::from_array(actor.position) + Vec3::Y * 2.35;
            let viewport = camera.world_to_viewport(camera_transform, position).ok()?;
            let x = viewport.x - safe_area.left();
            let y = viewport.y - safe_area.top();
            (x.is_finite()
                && y.is_finite()
                && x >= 0.0
                && x <= content_width
                && y >= 0.0
                && y <= content_height)
                .then_some(BelowNameAnchor {
                    x,
                    y,
                    name,
                    score: below_name.0,
                    objective: below_name.1,
                })
        })
        .take(retained_hud::MAX_PRESENTED_BELOW_NAME_ROWS)
        .collect()
}

/// Nametags for other players and flagged mobs; players with a below-name score get the combined
/// plate instead. Wall occlusion uses the collision store, failing open when it is unavailable.
fn project_nametags(
    scoreboards: &ui::ScoreboardStore,
    stream: &client_world::WorldStream,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    logical_size: [f32; 2],
    safe_area: SafeArea,
    collisions: Option<&crate::movement::PhysicsCollisionRegistries>,
) -> Vec<nametags::NametagAnchor> {
    let content_size = [
        (logical_size[0] - safe_area.left() - safe_area.right()).max(0.0),
        (logical_size[1] - safe_area.top() - safe_area.bottom()).max(0.0),
    ];
    let world = collisions.map(|collisions| {
        sim::PaletteWorld::new(
            stream.collision_store(),
            collisions.registry(stream.network_id_mode()),
            stream.current_dimension(),
        )
    });
    let eye = camera_transform.translation();
    let is_occluded = |target: Vec3| {
        let Some(world) = world.as_ref() else {
            return false;
        };
        let offset = target - eye;
        let distance = f64::from(offset.length());
        if !distance.is_finite() || distance <= 0.0 {
            return false;
        }
        let direction = offset.normalize();
        let vector = |value: Vec3| {
            sim::Vec3::new(f64::from(value.x), f64::from(value.y), f64::from(value.z))
        };
        matches!(
            world.block_interaction_ray_current(vector(eye), vector(direction), distance),
            Ok(Some(_))
        )
    };
    stream
        .remote_actors()
        .filter_map(|actor| {
            let scored = scoreboards
                .below_name_for_owner(&ui::ScoreOwner::Player(actor.unique_id))
                .or_else(|| {
                    scoreboards.below_name_for_owner(&ui::ScoreOwner::Entity(actor.unique_id))
                });
            if scored.is_some() {
                return None;
            }
            let name = stream.actor_display_name(actor.unique_id)?;
            nametags::project_nametag(
                actor,
                name,
                camera,
                camera_transform,
                content_size,
                safe_area,
                is_occluded,
            )
        })
        .take(nametags::MAX_PRESENTED_NAMETAGS)
        .collect()
}

impl UiPresentationRuntime {
    /// Returns the previous frame when only the revision would differ, so the
    /// renderer keeps its accepted publication and skips re-uploading.
    pub(super) fn stabilize_revision(&mut self, mut input: UiRenderInput) -> UiRenderInput {
        if let Some(previous) = &self.last_input {
            input.revision = previous.revision;
            if *previous == input {
                return previous.clone();
            }
        }
        self.revision = self.revision.saturating_add(1);
        input.revision = self.revision;
        self.last_input = Some(input.clone());
        input
    }
}
