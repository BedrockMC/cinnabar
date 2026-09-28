//! Bedrock 1.26.44 (protocol 2168) packet definitions and codec.

mod actor;
mod audio;
mod blob_cache;
mod camera;
mod codec;
mod disconnect;
mod interaction;
mod inventory;
mod item;
mod item_capacity;
mod login;
mod movement;
mod nbt_tree;
mod packet;
mod particle;
mod permissions;
mod raw_text;
mod socket_transport;
mod transfer;
mod ui;
mod world;

pub use actor::{
    ActorAttribute, ActorAttributeModifier, ActorAttributesUpdateEvent, ActorEffectAction,
    ActorEffectEvent, ActorEvent, ActorKind, ActorLinkEvent, ActorLinkType, ActorMetadata,
    ActorMetadataUpdateEvent, ActorMetadataValue, ActorMoveEvent, ActorPacketError,
    ActorPositionOrigin, ActorProperty, ActorRemoveEvent, ActorSpawnEvent, ActorStatusEvent,
    ActorStatusKind, ActorTakeItemEvent, MAX_ACTOR_ATTRIBUTE_MODIFIERS, MAX_ACTOR_ATTRIBUTES,
    MAX_ACTOR_IDENTIFIER_BYTES, MAX_ACTOR_LINKS_PER_SPAWN, MAX_ACTOR_METADATA_ENTRIES,
    MAX_ACTOR_METADATA_NBT_BYTES, MAX_ACTOR_METADATA_STRING_BYTES, MAX_ACTOR_NAME_BYTES,
    MAX_ACTOR_PROPERTIES, MAX_PLAYER_LIST_RECORDS, MAX_PLAYER_LIST_SKIN_BYTES,
    MAX_STANDARD_SKIN_SIDE, PlayerListEntry, PlayerListUpdateEvent, PlayerSkin,
    PlayerSkinUnavailable, StandardSkin,
};
pub use audio::{
    AudioEvent, LevelAudioEvent, MAX_AUDIO_IDENTIFIER_BYTES, PlayAudioEvent, StopAudioEvent,
};
pub use blob_cache::{
    BlobCacheError, BlobCacheLimits, BlobCacheReady, BlobCacheResolver, BlobCacheStats,
    BlobCacheStatus, CLIENT_BLOB_CACHE_TRIM_FLOOR_BYTES, CLIENT_BLOB_CACHE_TRIM_TRIGGER_BYTES,
    ClientBlobCache, MAX_CLIENT_BLOB_HASHES_PER_PACKET, MAX_CLIENT_BLOB_ORDINARY_READY_BYTES,
    MAX_CLIENT_BLOB_ORDINARY_READY_EVENTS, MAX_CLIENT_BLOB_PENDING_BYTES,
    MAX_CLIENT_BLOB_PENDING_TRANSACTIONS, MAX_CLIENT_BLOB_READY_BYTES,
    MAX_CLIENT_BLOB_RECONSTRUCTED_BYTES, MAX_CLIENT_BLOB_RECOVERY_READY_EVENTS,
    MAX_CLIENT_BLOB_STAGED_BYTES_PER_TRANSACTION, client_blob_hash,
};
pub use camera::{
    CameraEase, CameraEvent, CameraFadeColor, CameraFadeInstruction, CameraFadeTimes,
    CameraFovInstruction, CameraInstructionEvent, CameraPreset, CameraSetInstruction,
    CameraShakeAction, CameraShakeEvent, CameraShakeType, CameraSwitchEvent,
    CameraTargetInstruction, MAX_CAMERA_EASE_IDENTIFIER_BYTES, MAX_CAMERA_PRESETS,
};
pub use codec::{ProtocolError, decode_batch, encode};
pub use disconnect::ServerDisconnectEvent;
pub use interaction::{
    ActorUseAction, ActorUsePacketError, ActorUseRequest, BlockUsePacketError, BlockUseRequest,
    ItemUseTrigger, SwingSource, click_block_packet, click_block_transaction_packet,
    destroy_block_packet, swing_arm_packet, use_actor_packet,
};
pub use inventory::recipes::{
    CraftGridItem, CraftGridMatch, RECIPE_OWNED_BYTES, RecipeCatalog, RecipeHandle, RecipeOutput,
    RecipeUpdate, decode_recipe_update, match_crafting_grid,
};
pub use inventory::{
    ARMOR_SLOTS, ARMOR_WINDOW_ID, AutoCraftIngredient, CONTAINER_NAME_CREATED_OUTPUT,
    CONTAINER_NAME_HOTBAR, CRAFTING_INPUT_SLOTS, CREATED_OUTPUT_SLOT, ContainerWindow, CraftResult,
    LAST_CONTAINER_NAME, MAX_STACK_REQUEST_ACTIONS, StackItemDescriptor, container_window,
    is_personal_ui_inventory,
};
pub use inventory::{
    CONTAINER_NAME_ARMOR, CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY, CONTAINER_NAME_CRAFT_INPUT,
    CONTAINER_NAME_CURSOR, CONTAINER_NAME_INVENTORY, CONTAINER_NAME_LEVEL_ENTITY,
    CONTAINER_NAME_OFFHAND, CanonicalCell, ContainerCloseEvent, ContainerDataEvent,
    ContainerIdentity, ContainerOpenEvent, InventoryAuthority, InventoryContentEvent,
    InventoryEvent, InventoryPacketError, InventorySlotEvent, ItemStackResponseEvent,
    MAX_CONTAINER_SLOTS, MAX_ITEM_NBT_BYTES, MAX_RESPONSE_CONTAINERS, MAX_RESPONSE_NAME_BYTES,
    MAX_STACK_RESPONSES, OFFHAND_WINDOW_ID, PLAYER_INVENTORY_SLOTS, PLAYER_INVENTORY_WINDOW_ID,
    SelectedSlotEvent, SlotIdentity, StackRequestAction, StackRequestContainer, StackRequestSlot,
    StackResponse, StackResponseContainer, StackResponseSlot, StackResponseStatus,
    VerifiedNetworkItemStack, container_close_packet, item_stack_request_packet,
    normalize_authority, normalize_container_close, normalize_container_data,
    normalize_container_open, normalize_content, normalize_hotbar, normalize_response,
    normalize_slot, open_inventory_packet, personal_craft_content_indices,
    personal_craft_slot_index, project_container_cell, validate_item_nbt_size,
};
pub use inventory::{
    CreativeCategory, CreativeContentEvent, CreativeGroup, CreativeItem, MAX_CREATIVE_GROUPS,
    MAX_CREATIVE_ITEMS,
};
pub use inventory::{
    IngredientObservation, MAX_RECIPE_OBSERVATIONS, RecipeObservation, RecipeObservations,
};
pub use inventory::{
    ManualCraftCell, ManualCraftMatch, ManualCraftPreview, RecipeRegistryError,
    RecipeRegistrySnapshot, match_manual_grid,
};
pub use inventory::{ManualCraftError, ManualCraftInput, ManualCraftSnapshot, manual_craft_packet};
pub use inventory::{MineBlockRequest, MineBlockRequestError};
pub use item::{
    ActorActionEvent, ActorActionKind, ActorHandedness, ArmorEquipmentEvent, EquipmentEvent,
    HOTBAR_SLOT_COUNT, ItemActorEvent, ItemPacketError, ItemRegistryEntry, ItemRegistryEvent,
    ItemRegistryVersion, MAX_ACTION_IDENTIFIER_BYTES, MAX_ANIMATE_ENTITY_IDS,
    MAX_ANIMATION_IDENTIFIER_BYTES, MAX_ITEM_EXTRA_BYTES, MAX_ITEM_REGISTRY_ENTRIES,
    NetworkItemStack, item_charged_projectile, item_custom_color, item_enchantment_level, item_extra_damage, item_icon_keys,
    item_stack_damage, select_hotbar_slot_packet, vanilla_item_registry,
};
pub use item_capacity::vanilla_item_capacity;
pub use jolyne::GameData;
pub use jolyne::stream::client::ClientSkin;
pub use jolyne::stream::{ResourcePackArchive, ResourcePackContentKey, ResourcePackHandoff};
pub use login::{LoginSequence, PacketIdTraceSnapshot, PlaySession};
pub use movement::{
    BlockAction, BlockActionKind, BlockActions, BlockActionsFull, BlockItemInteraction,
    InteractionEncodeError, MAX_BLOCK_ACTIONS_PER_INPUT, PlayerAuthInputError,
    PlayerAuthInputInteractions, PlayerAuthInputSnapshot, PlayerAuthInputTraceSample,
    PlayerInputFlags, PlayerInputMode, player_auth_input, player_auth_input_trace_sample,
    player_auth_input_with_interactions, player_auth_input_with_mining_request,
};
pub use packet::Packet;
pub use particle::{
    LevelParticleEvent, MAX_PARTICLE_NAME_BYTES, MAX_PARTICLE_VARIABLES_BYTES, ParticleEvent,
    SpawnParticleEffectEvent,
};
pub use permissions::{
    AbilitiesUpdate, AbilityLayerEvidence, AbilityLayersEvidence, MAX_ABILITY_LAYERS,
    decode_abilities_update,
};
pub use raw_text::{
    MAX_RAW_TEXT_COMPONENTS, MAX_RAW_TEXT_DEPTH, MAX_RAW_TEXT_INPUT_BYTES, MAX_RAW_TEXT_NODES,
    MAX_RAW_TEXT_OUTPUT_BYTES, RawTextComponent, RawTextDocument, RawTextResolution,
    RawTextResolver, ResolvedRawText, format_translation, parse_raw_text,
};
pub use socket_transport::{SocketTransport, bridge_endpoint_path, report_pack_application};
pub use transfer::{MAX_TRANSFER_HOST_BYTES, ServerTransferEvent, ServerTransferRejection};
pub use ui::{
    BlockCrackAction, BlockCrackEvent, BossAction, BossColor, BossEvent, BossOverlay, BossStyle,
    ChatAutocompleteAction, ChatAutocompleteCatalog, ChatAutocompleteCatalogError,
    ChatAutocompleteCompletion, ChatAutocompleteEvent, ChatPacketError, CommandOutputEvent,
    CommandOutputMessage, CustomFormValue, FormButtonImage, FormKind, FormRequestEvent,
    GameModeEvent, GameModeUpdate, HudEvent, MAX_BOSS_EVENTS, MAX_CHAT_AUTOCOMPLETE,
    MAX_CHAT_AUTOCOMPLETE_BYTES, MAX_CHAT_PARAMETERS, MAX_COMMAND_OUTPUT_MESSAGES,
    MAX_FORM_BUTTONS, MAX_FORM_JSON_BYTES, MAX_FORM_JSON_DEPTH, MAX_OUTBOUND_CHAT_BYTES,
    MAX_SCORE_ENTRIES_PER_PACKET, MAX_UI_TEXT_BYTES, ModalFormResponseSelection, ObjectiveEvent,
    PlayerStatus, RawTextEvent, ScoreAction, ScoreEntry, ScoreEvent, ScoreIdentity,
    ServerFormModel, TextCategory, TextEvent, TextKind, TextMenuForm, TitleAction, TitleEvent,
    UiEvent, UiPacketError, UnsupportedForm, chat_input_packet, chat_text_packet,
    custom_form_submit_response, modal_form_busy_response, modal_form_cancel_response,
    modal_form_submit_response,
};
pub use valentine::bedrock::context::BedrockSession;
pub use valentine::bedrock::version::v1_26_44::{GAME_VERSION, PROTOCOL_VERSION};
pub use world::{
    ActorMotionEvent, BiomeDefinitionEvent, BiomeDefinitionsEvent, BlockEntityUpdateEvent,
    BlockEventEvent, BlockUpdateEvent, ChangeDimensionEvent, ChunkResyncEvent, CustomBlock,
    CustomBlockVisuals, CustomBlocks, CustomBox, CustomHashedState, CustomMaterialInstance,
    CustomPermutation, CustomStateAxis, CustomStateValue, CustomTransformation,
    CustomVisualComponents, DaylightCycleUpdateEvent, DimensionRange, HASHED_AIR_NETWORK_ID,
    LevelChunkEvent, LevelChunkMode, MAX_BIOME_DEFINITIONS, MAX_BIOME_NAME_BYTES, MAX_BLOCK_LAYERS,
    MAX_SUB_CHUNK_REQUESTS, MovePlayerEvent, MovePlayerMode, MovementCorrectionSubject,
    PLAYER_NETWORK_OFFSET, PlayerGameMode, PlayerMovementCorrectionEvent, PublisherUpdateEvent,
    RespawnEvent, SEQUENTIAL_AIR_NETWORK_ID, STANDING_PLAYER_EYE_HEIGHT, SetTimeEvent,
    SubChunkBatchEvent, SubChunkEntryEvent, SubChunkReplyAdmissionEvent, SubChunkResult,
    SubChunkUnavailable, WeatherChannel, WeatherUpdateEvent, WorldBootstrap,
    WorldEnvironmentBootstrap, WorldEvent, WorldPacketError, WorldWireError, air_network_id,
    block_name_sort_key, into_world_event, request_sub_chunk_column,
    server_authoritative_block_breaking, vanilla_dimension_range,
};
