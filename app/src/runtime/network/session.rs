use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use bevy::prelude::Resource;
use bytes::Bytes;
use protocol::{
    BlobCacheStats, ClientBlobCache, CustomBlocks, InventoryEvent, ItemRegistryEvent,
    LoginSequence, Packet, PacketIdTraceSnapshot, PlayerGameMode, ServerDisconnectEvent,
    WorldBootstrap, WorldEnvironmentBootstrap, WorldEvent,
};
use tokio::sync::{mpsc, watch};
use world::ChunkKey;

use crate::{
    acceptance::mutation::write_stdout_marker,
    movement::{InteractionPacketGuard, MovementTicker, PhysicsSendIdentity},
    ui_runtime::FastTransferAction,
};

pub(crate) const WORLD_EVENT_CAPACITY: usize = 32;
const CONTROL_EVENT_CAPACITY: usize = 64;
const COMMAND_CAPACITY: usize = 64;
const FINAL_CONTROL_FLUSH_TIMEOUT: Duration = Duration::from_millis(250);
const NETWORK_PUMP_TERMINAL_MARKER: &str = "RUST_MCBE_NETWORK_PUMP_TERMINAL";

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub session_generation: u64,
    pub socket_dir: PathBuf,
    pub display_name: String,
    /// Verified blobs outlive a Play session; each login creates a fresh resolver around this cache.
    pub client_blob_cache: ClientBlobCache,
    /// The client's own skin, uploaded in the ClientData login payload.
    pub player_skin: crate::player_skin::LocalPlayerSkin,
}

/// Which transport leg or lifecycle stage produced a session failure.
///
/// Only [`NetworkFailureOrigin::Receive`] represents a remote-initiated
/// termination of an active play session; every other origin is owned by the
/// local process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkFailureOrigin {
    /// The active session's inbound transport failed: the server disconnected
    /// or the upstream read terminated mid-session.
    Receive,
    /// The outbound transport failed while writing a packet.
    Send,
    /// The session failed before the play pump started (runtime/login).
    Startup,
}

/// The bounded server-directed transfer target carried by terminal events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionTransferTarget {
    pub(crate) host: String,
    pub(crate) port: u16,
}

#[derive(Debug)]
pub enum NetworkControlEvent {
    Bootstrap {
        session_generation: u64,
        world: WorldBootstrap,
        environment: WorldEnvironmentBootstrap,
        custom_blocks: CustomBlocks,
        inventory: InventoryEvent,
        item_registry: Option<ItemRegistryEvent>,
        player_game_mode: PlayerGameMode,
        world_default_game_mode: PlayerGameMode,
        player_game_mode_uses_world_default: bool,
        server_authoritative_block_breaking: bool,
        /// StartGame `RewindHistorySize`, raw.
        rewind_history_size: i32,
        hardcore: bool,
        hud_rules: protocol::HudRules,
        packs: super::resource_packs::PackApplication,
    },
    SubChunkRequestSent {
        chunk: ChunkKey,
        base_sub_chunk_y: i32,
        count: usize,
        sent_at: Instant,
    },
    ChatPacketSent {
        session: u64,
        sequence: u64,
    },
    ChatPacketSendFailed {
        session: u64,
        sequence: u64,
        message: String,
    },
    PhysicsPacketSent {
        identity: PhysicsSendIdentity,
    },
    PhysicsPacketCancelled {
        identity: PhysicsSendIdentity,
        definitely_unsent: bool,
    },
    BlobCacheTelemetry {
        enabled: bool,
        stats: BlobCacheStats,
    },
    Failed {
        message: String,
        decode_error_count: u64,
        server_disconnect: Option<ServerDisconnectEvent>,
        origin: NetworkFailureOrigin,
    },
    /// The server directed the client to a new target, ending this session.
    ///
    /// Like [`NetworkControlEvent::Failed`] this is terminal: the pump stops
    /// after emitting it and no `Stopped` record follows. The wire's
    /// `reload_world` hint is recorded in the pump's durable transferred
    /// marker; it does not change the app-side handoff.
    Transferred {
        target: SessionTransferTarget,
        decode_error_count: u64,
    },
    Stopped {
        decode_error_count: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SequencedWorldEvent {
    pub session_generation: u64,
    pub sequence: u64,
    pub event: WorldEvent,
}

#[derive(Debug, Clone, PartialEq)]
enum InboundWorldEvent {
    Event(WorldEvent),
    LevelChunk {
        event: protocol::LevelChunkEvent,
        payload: Bytes,
    },
}

#[derive(Debug, Clone, PartialEq)]
// The FIFO is strictly bounded to WORLD_EVENT_CAPACITY. Keeping the event
// inline avoids adding one heap allocation to every normal world packet just
// to accommodate the rare, small transfer barrier variant.
#[allow(clippy::large_enum_variant)]
pub enum WorldIngress {
    Event(SequencedWorldEvent),
    LevelChunk {
        session_generation: u64,
        sequence: u64,
        event: protocol::LevelChunkEvent,
        payload: Bytes,
    },
    FastTransferBarrier {
        session_generation: u64,
        sequence: u64,
        action_sequence: u64,
    },
}

#[derive(Debug, Default)]
struct ReadinessIngressCounter {
    produced: AtomicU64,
    consumed: AtomicU64,
}

impl ReadinessIngressCounter {
    fn record_produced(&self, event: &WorldEvent) {
        if readiness_affecting_world_event(event) {
            self.produced.fetch_add(1, Ordering::Release);
        }
    }

    fn record_consumed(&self, event: &WorldEvent) {
        if readiness_affecting_world_event(event) {
            self.consumed.fetch_add(1, Ordering::Release);
        }
    }

    fn progress(&self) -> (u64, u64) {
        (
            self.produced.load(Ordering::Acquire),
            self.consumed.load(Ordering::Acquire),
        )
    }

    fn pending(&self) -> usize {
        let (produced, consumed) = self.progress();
        usize::try_from(produced.saturating_sub(consumed)).unwrap_or(usize::MAX)
    }
}

fn readiness_affecting_world_event(event: &WorldEvent) -> bool {
    matches!(
        event,
        WorldEvent::BiomeDefinitions(_)
            | WorldEvent::LevelChunk(_)
            | WorldEvent::ChunkResync(_)
            | WorldEvent::SubChunkReplyAdmission(_)
            | WorldEvent::SubChunks(_)
            | WorldEvent::BlockUpdates(_)
            | WorldEvent::BlockEntityUpdate(_)
            | WorldEvent::ChunkRadiusUpdated(_)
            | WorldEvent::PublisherUpdate(_)
            | WorldEvent::ChangeDimension(_)
            | WorldEvent::Respawn(_)
            | WorldEvent::MovePlayer(_)
            | WorldEvent::PlayerMovementCorrection(_)
    )
}

fn wrap_readiness_tracked_event(
    sequencer: &mut NetworkSequencer,
    readiness_ingress: &ReadinessIngressCounter,
    event: WorldEvent,
) -> SequencedWorldEvent {
    let sequenced = sequencer.wrap(event);
    readiness_ingress.record_produced(&sequenced.event);
    sequenced
}

fn wrap_inbound_world_event(
    sequencer: &mut NetworkSequencer,
    readiness_ingress: &ReadinessIngressCounter,
    event: InboundWorldEvent,
) -> WorldIngress {
    match event {
        InboundWorldEvent::Event(event) => WorldIngress::Event(wrap_readiness_tracked_event(
            sequencer,
            readiness_ingress,
            event,
        )),
        InboundWorldEvent::LevelChunk { event, payload } => {
            let sequence = sequencer.take_sequence();
            readiness_ingress.produced.fetch_add(1, Ordering::Release);
            WorldIngress::LevelChunk {
                session_generation: sequencer.session_generation(),
                sequence,
                event,
                payload,
            }
        }
    }
}

#[derive(Debug)]
enum NetworkCommand {
    Send {
        packet: Packet,
        sub_chunk: Option<SubChunkRequestSend>,
        chat: Option<ChatPacketSend>,
        physics: Option<PhysicsSendIdentity>,
        physics_reanchor: Option<watch::Receiver<u64>>,
        interaction: Option<InteractionPacketGuard>,
    },
}

#[derive(Debug, Clone, Copy)]
struct SubChunkRequestSend {
    chunk: ChunkKey,
    base_sub_chunk_y: i32,
    count: usize,
}

#[derive(Debug, Clone, Copy)]
struct ChatPacketSend {
    session: u64,
    sequence: u64,
    fast_transfer_action: Option<FastTransferAction>,
}

#[derive(Debug)]
pub enum PacketSendError {
    Full(Packet),
    Closed(Packet),
}

impl std::fmt::Display for PacketSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full(_) => formatter.write_str("network command queue is full"),
            Self::Closed(_) => formatter.write_str("network command channel is closed"),
        }
    }
}

impl std::error::Error for PacketSendError {}

impl PacketSendError {
    #[must_use]
    pub fn into_packet(self) -> Packet {
        match self {
            Self::Full(packet) | Self::Closed(packet) => packet,
        }
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        matches!(self, Self::Closed(_))
    }
}

/// Why a packet batch was not queued; nothing from it was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchSendError {
    Full,
    Closed,
}

#[derive(Resource)]
pub struct NetworkHandle {
    session_generation: u64,
    control_events: mpsc::Receiver<NetworkControlEvent>,
    world_events: mpsc::Receiver<WorldIngress>,
    commands: mpsc::Sender<NetworkCommand>,
    physics_reanchor: watch::Sender<u64>,
    shutdown: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
    readiness_ingress: Arc<ReadinessIngressCounter>,
}

impl NetworkHandle {
    /// A live Bevy app still owns a network resource while it is sitting at
    /// the launcher. Empty channels keep every production system total and
    /// let the menu start without a Go bridge; a later menu connection can
    /// replace this resource with a real session.
    pub(crate) fn disconnected() -> Self {
        empty_network_channels().0
    }

    #[cfg(test)]
    pub(crate) fn stub() -> (Self, watch::Receiver<u64>) {
        empty_network_channels()
    }
    /// A handle whose command queue holds `capacity` commands; the guard keeps it open.
    #[cfg(test)]
    pub(crate) fn with_command_capacity(capacity: usize) -> (Self, Box<dyn std::any::Any>) {
        let (mut handle, _) = empty_network_channels();
        let (commands, receiver) = mpsc::channel::<NetworkCommand>(capacity);
        handle.commands = commands;
        (handle, Box::new(receiver))
    }
    #[cfg(test)]
    pub(crate) fn shutdown_requested(&self) -> bool {
        *self.shutdown.borrow()
    }

    pub(crate) fn movement_ticker(&self) -> MovementTicker {
        MovementTicker::with_epoch_publisher(self.physics_reanchor.clone())
    }

    pub fn control_events_mut(&mut self) -> &mut mpsc::Receiver<NetworkControlEvent> {
        &mut self.control_events
    }

    pub fn world_events_mut(&mut self) -> &mut mpsc::Receiver<WorldIngress> {
        &mut self.world_events
    }

    #[must_use]
    pub fn pending_event_count(&self) -> usize {
        self.control_events
            .len()
            .saturating_add(self.world_events.len())
    }

    #[must_use]
    pub fn pending_command_count(&self) -> usize {
        self.commands
            .max_capacity()
            .saturating_sub(self.commands.capacity())
    }

    #[must_use]
    pub(crate) fn pending_readiness_event_count(&self) -> usize {
        self.readiness_ingress.pending()
    }

    #[must_use]
    pub(crate) fn readiness_ingress_progress(&self) -> (u64, u64) {
        self.readiness_ingress.progress()
    }

    pub(crate) fn record_readiness_event_consumed(&self, event: &WorldEvent) {
        self.readiness_ingress.record_consumed(event);
    }

    pub(crate) fn record_level_chunk_consumed(&self) {
        self.readiness_ingress
            .consumed
            .fetch_add(1, Ordering::Release);
    }

    pub(crate) fn send_physics_packet(
        &self,
        identity: PhysicsSendIdentity,
        packet: Packet,
        interaction: Option<InteractionPacketGuard>,
    ) -> Result<(), PacketSendError> {
        self.send_packet_with_confirmation(
            packet,
            None,
            None,
            Some(identity),
            Some(self.physics_reanchor.subscribe()),
            interaction,
        )
    }

    pub(crate) fn send_hotbar_packet(&self, packet: Packet) -> Result<(), PacketSendError> {
        self.send_packet_with_confirmation(packet, None, None, None, None, None)
    }

    /// Queues an unguarded movement-side packet such as a prediction sync.
    pub(crate) fn send_movement_packet(&self, packet: Packet) -> Result<(), PacketSendError> {
        self.send_packet_with_confirmation(packet, None, None, None, None, None)
    }

    /// Queues an inventory, swing or interaction packet ahead of this frame's movement.
    pub(crate) fn send_inventory_packet(&self, packet: Packet) -> Result<(), PacketSendError> {
        self.send_packet_with_confirmation(packet, None, None, None, None, None)
    }

    /// Queues standalone packets together or not at all, so a swing never leaves without its
    /// transaction.
    pub(crate) fn send_inventory_packets(
        &self,
        packets: Vec<Packet>,
    ) -> Result<(), BatchSendError> {
        if packets.is_empty() {
            return Ok(());
        }
        let permits =
            self.commands
                .try_reserve_many(packets.len())
                .map_err(|error| match error {
                    mpsc::error::TrySendError::Full(()) => BatchSendError::Full,
                    mpsc::error::TrySendError::Closed(()) => BatchSendError::Closed,
                })?;
        for (permit, packet) in permits.zip(packets) {
            permit.send(NetworkCommand::Send {
                packet,
                sub_chunk: None,
                chat: None,
                physics: None,
                physics_reanchor: None,
                interaction: None,
            });
        }
        Ok(())
    }

    pub fn send_chat_packet(
        &self,
        session: u64,
        sequence: u64,
        fast_transfer_action: Option<FastTransferAction>,
        packet: Packet,
    ) -> Result<(), PacketSendError> {
        self.send_packet_with_confirmation(
            packet,
            None,
            Some(ChatPacketSend {
                session,
                sequence,
                fast_transfer_action,
            }),
            None,
            None,
            None,
        )
    }

    pub fn send_sub_chunk_request(
        &self,
        chunk: ChunkKey,
        base_sub_chunk_y: i32,
        count: usize,
        packet: Packet,
    ) -> Result<(), PacketSendError> {
        self.send_packet_with_confirmation(
            packet,
            Some(SubChunkRequestSend {
                chunk,
                base_sub_chunk_y,
                count,
            }),
            None,
            None,
            None,
            None,
        )
    }

    fn send_packet_with_confirmation(
        &self,
        packet: Packet,
        sub_chunk: Option<SubChunkRequestSend>,
        chat: Option<ChatPacketSend>,
        physics: Option<PhysicsSendIdentity>,
        physics_reanchor: Option<watch::Receiver<u64>>,
        interaction: Option<InteractionPacketGuard>,
    ) -> Result<(), PacketSendError> {
        self.commands
            .try_send(NetworkCommand::Send {
                packet,
                sub_chunk,
                chat,
                physics,
                physics_reanchor,
                interaction,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(NetworkCommand::Send { packet, .. }) => {
                    PacketSendError::Full(packet)
                }
                mpsc::error::TrySendError::Closed(NetworkCommand::Send { packet, .. }) => {
                    PacketSendError::Closed(packet)
                }
            })
    }

    pub fn shutdown(&mut self) {
        self.shutdown.send_replace(true);
        self.release_thread();
    }

    fn release_thread(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        if thread.is_finished() {
            let _ = thread.join();
            return;
        }
        // Joining can wait on socket teardown or a slow transport. Keep that
        // wait off Bevy's UI thread while still reaping the worker normally.
        let _ = thread::Builder::new()
            .name("bedrock-network-reaper".to_owned())
            .spawn(move || {
                let _ = thread.join();
            });
    }
}

fn empty_network_channels() -> (NetworkHandle, watch::Receiver<u64>) {
    let (_control_event_tx, control_events) = mpsc::channel(1);
    let (_world_event_tx, world_events) = mpsc::channel(1);
    let (commands, _command_rx) = mpsc::channel(1);
    let (physics_reanchor, physics_reanchor_rx) = watch::channel(0);
    let (shutdown, _shutdown_rx) = watch::channel(false);
    (
        NetworkHandle {
            session_generation: 0,
            control_events,
            world_events,
            commands,
            physics_reanchor,
            shutdown,
            thread: None,
            readiness_ingress: Arc::new(ReadinessIngressCounter::default()),
        },
        physics_reanchor_rx,
    )
}

impl Drop for NetworkHandle {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        self.release_thread();
    }
}

pub fn spawn_network(config: NetworkConfig) -> Result<NetworkHandle, std::io::Error> {
    let session_generation = config.session_generation;
    let (control_event_tx, control_events) = mpsc::channel(CONTROL_EVENT_CAPACITY);
    let (world_event_tx, world_events) = mpsc::channel(WORLD_EVENT_CAPACITY);
    let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
    let (physics_reanchor, _physics_reanchor_rx) = watch::channel(0);
    let (shutdown, mut shutdown_rx) = watch::channel(false);
    let readiness_ingress = Arc::new(ReadinessIngressCounter::default());
    let network_readiness_ingress = Arc::clone(&readiness_ingress);
    let thread = thread::Builder::new()
        .name("bedrock-network".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = control_event_tx.try_send(NetworkControlEvent::Failed {
                        message: format!("failed to create network runtime: {error}"),
                        decode_error_count: 0,
                        server_disconnect: None,
                        origin: NetworkFailureOrigin::Startup,
                    });
                    return;
                }
            };
            runtime.block_on(async move {
                let Some(login) = wait_for_login_or_cancel(
                    LoginSequence::connect_with_blob_cache(
                        &config.socket_dir,
                        &config.display_name,
                        config.client_blob_cache.clone(),
                        Some(config.player_skin.to_client_skin()),
                    ),
                    &mut shutdown_rx,
                )
                .await
                else {
                    return;
                };
                let (mut session, game_data) = match login {
                    Ok(connected) => connected,
                    Err(error) => {
                        // A join-time Disconnect words the failure like a play-time one.
                        let disconnect = error.server_disconnect();
                        send_startup_failure(
                            &control_event_tx,
                            &mut shutdown_rx,
                            error,
                            disconnect,
                        )
                        .await;
                        return;
                    }
                };
                // The login handoff is one-shot. Take and validate it before
                // publishing any StartGame state; optional semantic rejection
                // remains a live base-assets session.
                let handoff = session.take_resource_pack_handoff();
                let (custom_blocks, packs) =
                    super::resource_packs::prepare_session_packs(handoff, &game_data);
                let packs_applied = matches!(
                    &packs.admission,
                    resource_pack::PackAdmission::Validated(stack) if !stack.packs().is_empty()
                );
                if packs_applied {
                    let socket_dir = config.socket_dir.clone();
                    tokio::spawn(async move {
                        protocol::report_pack_application(&socket_dir, true).await;
                    });
                }
                let bootstrap = WorldBootstrap::from_game_data(&game_data);
                let server_authoritative_block_breaking =
                    protocol::server_authoritative_block_breaking(&game_data);
                let environment = WorldEnvironmentBootstrap::from_game_data(&game_data);
                let hardcore = protocol::is_hardcore(&game_data);
                let hud_rules = protocol::HudRules::from_game_data(&game_data);
                let inventory = start_game_inventory_authority(&game_data);
                let item_registry = match start_game_item_registry(&game_data, bootstrap.dimension)
                {
                    Ok(registry) => registry,
                    Err(error) => {
                        send_startup_failure(&control_event_tx, &mut shutdown_rx, error, None)
                            .await;
                        return;
                    }
                };
                let player_game_mode = PlayerGameMode::from_game_data(&game_data);
                let world_default_game_mode =
                    PlayerGameMode::world_default_from_game_data(&game_data);
                let player_game_mode_uses_world_default =
                    PlayerGameMode::bootstrap_uses_world_default(&game_data);
                if !send_control_event_or_cancel(
                    &control_event_tx,
                    &mut shutdown_rx,
                    NetworkControlEvent::Bootstrap {
                        session_generation,
                        world: bootstrap,
                        environment,
                        custom_blocks,
                        inventory,
                        item_registry,
                        player_game_mode,
                        world_default_game_mode,
                        player_game_mode_uses_world_default,
                        server_authoritative_block_breaking,
                        rewind_history_size: protocol::rewind_history_size(&game_data),
                        hardcore,
                        hud_rules,
                        packs,
                    },
                )
                .await
                {
                    return;
                }
                let sequencer = NetworkSequencer::new(
                    session_generation,
                    bootstrap.dimension,
                    bootstrap.local_player_runtime_id,
                );
                run_network_pump_with_readiness_ingress(
                    session,
                    sequencer,
                    command_rx,
                    control_event_tx,
                    world_event_tx,
                    shutdown_rx,
                    network_readiness_ingress,
                )
                .await;
                if packs_applied {
                    protocol::report_pack_application(&config.socket_dir, false).await;
                }
            });
        })?;
    Ok(NetworkHandle {
        session_generation,
        control_events,
        world_events,
        commands,
        physics_reanchor,
        shutdown,
        thread: Some(thread),
        readiness_ingress,
    })
}

trait NetworkSession: Send {
    type Error: std::fmt::Display + Send;

    fn receive_world_event(
        &mut self,
        current_dimension: i32,
    ) -> impl Future<Output = Result<WorldEvent, Self::Error>> + Send;

    fn receive_world_ingress(
        &mut self,
        current_dimension: i32,
    ) -> impl Future<Output = Result<InboundWorldEvent, Self::Error>> + Send {
        async move {
            self.receive_world_event(current_dimension)
                .await
                .map(InboundWorldEvent::Event)
        }
    }

    fn send_packet(
        &mut self,
        packet: Packet,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn decode_error_count(&self) -> u64;

    fn take_server_disconnect(&mut self) -> Option<ServerDisconnectEvent> {
        None
    }

    fn take_server_transfer(&mut self) -> Option<protocol::ServerTransferEvent> {
        None
    }

    fn blob_cache_enabled(&self) -> bool {
        false
    }

    fn blob_cache_stats(&self) -> BlobCacheStats {
        BlobCacheStats::default()
    }

    fn begin_packet_id_trace(&mut self) {}

    fn cancel_packet_id_trace(&mut self) {}

    fn arm_blob_cache_reset_for_fast_transfer(&mut self) {}

    fn drain_packet_id_trace(&mut self) -> Option<PacketIdTraceSnapshot> {
        None
    }
}

impl NetworkSession for protocol::PlaySession {
    type Error = protocol::ProtocolError;

    fn receive_world_event(
        &mut self,
        current_dimension: i32,
    ) -> impl Future<Output = Result<WorldEvent, Self::Error>> + Send {
        self.recv_world_event(current_dimension)
    }

    fn receive_world_ingress(
        &mut self,
        current_dimension: i32,
    ) -> impl Future<Output = Result<InboundWorldEvent, Self::Error>> + Send {
        self.recv_world_event_mapped(
            current_dimension,
            InboundWorldEvent::Event,
            |event, payload| InboundWorldEvent::LevelChunk { event, payload },
        )
    }

    fn send_packet(
        &mut self,
        packet: Packet,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.send(packet)
    }

    fn decode_error_count(&self) -> u64 {
        protocol::PlaySession::decode_error_count(self)
    }

    fn take_server_disconnect(&mut self) -> Option<ServerDisconnectEvent> {
        protocol::PlaySession::take_server_disconnect(self)
    }

    fn take_server_transfer(&mut self) -> Option<protocol::ServerTransferEvent> {
        protocol::PlaySession::take_server_transfer(self)
    }

    fn blob_cache_enabled(&self) -> bool {
        protocol::PlaySession::blob_cache_enabled(self)
    }

    fn blob_cache_stats(&self) -> BlobCacheStats {
        protocol::PlaySession::blob_cache_stats(self)
    }

    fn begin_packet_id_trace(&mut self) {
        protocol::PlaySession::begin_packet_id_trace(self);
    }

    fn cancel_packet_id_trace(&mut self) {
        protocol::PlaySession::cancel_packet_id_trace(self);
    }

    fn arm_blob_cache_reset_for_fast_transfer(&mut self) {
        protocol::PlaySession::arm_blob_cache_reset_for_fast_transfer(self);
    }

    fn drain_packet_id_trace(&mut self) -> Option<PacketIdTraceSnapshot> {
        protocol::PlaySession::drain_packet_id_trace(self)
    }
}

pub(crate) fn session_failure_display(
    transport_message: &str,
    server_disconnect: Option<&ServerDisconnectEvent>,
) -> String {
    match server_disconnect.and_then(disconnect_display_reason) {
        Some(reason) => format!("server disconnected: {reason} ({transport_message})"),
        None => format!("network session failed: {transport_message}"),
    }
}

fn emit_network_pump_terminal_marker(
    stage: &'static str,
    message: &str,
    decode_errors: u64,
    server_disconnect: Option<&ServerDisconnectEvent>,
) {
    let mut stdout = std::io::stdout().lock();
    write_network_pump_terminal_marker(
        &mut stdout,
        stage,
        message,
        decode_errors,
        server_disconnect,
    );
    let _ = stdout.flush();
}

/// Emits the durable transferred-session record so live evidence attributes
/// the session end to the server's transfer instead of a transport failure.
fn emit_network_pump_transfer_marker(
    target: &SessionTransferTarget,
    reload_world: bool,
    decode_errors: u64,
) {
    let mut stdout = std::io::stdout().lock();
    write_network_pump_transfer_marker(&mut stdout, target, reload_world, decode_errors);
    let _ = stdout.flush();
}

fn write_network_pump_terminal_marker(
    writer: &mut impl Write,
    stage: &'static str,
    message: &str,
    decode_errors: u64,
    server_disconnect: Option<&ServerDisconnectEvent>,
) {
    let mut marker = serde_json::json!({
        "schema": "rust-mcbe-network-pump-terminal-v1",
        "outcome": "failed",
        "stage": stage,
        "message": message,
        "decode_error_count": decode_errors,
    });
    if let Some(disconnect) = server_disconnect {
        marker["server_disconnect"] = serde_json::json!({
            "reason": disconnect.reason,
            "message": disconnect.message,
            "filtered_message": disconnect.filtered_message,
        });
    }
    let _ = writeln!(writer, "{NETWORK_PUMP_TERMINAL_MARKER}={marker}");
}

fn write_network_pump_transfer_marker(
    writer: &mut impl Write,
    target: &SessionTransferTarget,
    reload_world: bool,
    decode_errors: u64,
) {
    let marker = serde_json::json!({
        "schema": "rust-mcbe-network-pump-terminal-v1",
        "outcome": "transferred",
        "target": { "host": target.host, "port": target.port },
        "reload_world": reload_world,
        "decode_error_count": decode_errors,
    });
    let _ = writeln!(writer, "{NETWORK_PUMP_TERMINAL_MARKER}={marker}");
}

fn emit_packet_id_trace<S: NetworkSession>(session: &mut S) {
    let Some(trace) = session.drain_packet_id_trace() else {
        return;
    };
    let marker = serde_json::json!({
        "schema": "rust-mcbe-fast-transfer-packet-trace-v1",
        "packet_ids": trace.packet_ids,
        "overflow": trace.overflow,
        "timed_out": trace.timed_out,
    });
    write_stdout_marker(
        &mut std::io::stdout().lock(),
        &format!(
            "{}={marker}",
            crate::acceptance::markers::FAST_TRANSFER_PACKET_TRACE
        ),
    );
}

mod blob_cache_telemetry;
#[cfg(test)]
use blob_cache_telemetry::bounded_counter_log_due;
use blob_cache_telemetry::{
    emit_blob_cache_telemetry, send_final_blob_cache_telemetry, try_emit_blob_cache_telemetry,
};
mod bootstrap;
mod forms;
mod handle_state;
use bootstrap::{send_startup_failure, start_game_inventory_authority, start_game_item_registry};
mod pump;
use pump::*;
mod pump_runtime;
use pump_runtime::*;
mod disconnect_display;
use disconnect_display::disconnect_display_reason;
#[cfg(test)]
mod tests;
