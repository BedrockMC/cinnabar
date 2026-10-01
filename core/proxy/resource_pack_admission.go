package proxy

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"sync"
	"sync/atomic"

	"github.com/google/uuid"
	"github.com/hashimthearab/rust-mcbe/core/internal/streamnet"
	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
	"github.com/sandertv/gophertunnel/minecraft/resource"
	"golang.org/x/oauth2"
)

// PackAdmissionFailureReason classifies a resource-pack failure without
// exposing pack identifiers, URLs, or content keys.
type PackAdmissionFailureReason uint8

const (
	// PackAdmissionRequiredUnsupported means the upstream requires one or more
	// packs that Cinnabar cannot truthfully apply yet.
	PackAdmissionRequiredUnsupported PackAdmissionFailureReason = iota + 1

	maxSelectedResourcePacks          = 32
	maxSelectedResourcePackTotalBytes = 128 * 1024 * 1024
	maxResourcePackArchiveBytes       = 64 * 1024 * 1024
	// Transfers may claim more bytes than their offer, so downloads are bounded
	// separately: gophertunnel holds every downloaded pack in memory until the
	// handoff is captured, and past this ceiling the dial is cancelled.
	maxResourcePackTransferBytes = 2 * maxSelectedResourcePackTotalBytes
)

// Like vanilla, a slow or silent pack download is never cancelled here: it ends when the
// server finishes, the user cancels, or the client's login deadline passes.
var errResourcePackTransferTooLarge = errors.New("proxy: resource-pack transfers exceeded their memory bound")

// ConnectStage names the vanilla progress handler a join is in.
type ConnectStage string

const (
	ConnectStageRealm      ConnectStage = "realm"      // RealmsConnectProgressHandler: the Realm lookup
	ConnectStageConnecting ConnectStage = "connecting" // GameServerConnectProgressHandler
	ConnectStagePacks      ConnectStage = "packs"      // ResourcePackProgressHandler
)

// ConnectProgress is the join's live stage; a zero Stage means no join is being prepared.
// Like vanilla, TotalBytes grows as each pack's download begins and PacksTotal excludes cache hits.
type ConnectProgress struct {
	Stage         ConnectStage `json:"stage"`
	PacksDone     uint32       `json:"packs_done,omitempty"`
	PacksTotal    uint32       `json:"packs_total,omitempty"`
	ReceivedBytes uint64       `json:"received_bytes,omitempty"`
	TotalBytes    uint64       `json:"total_bytes,omitempty"`
}

type connectProgressKey struct{}

// withConnectProgress lets target resolution report its stage.
func withConnectProgress(ctx context.Context, report func(ConnectProgress)) context.Context {
	return context.WithValue(ctx, connectProgressKey{}, report)
}

func reportConnectStage(ctx context.Context, stage ConnectStage) {
	if report, ok := ctx.Value(connectProgressKey{}).(func(ConnectProgress)); ok && report != nil {
		report(ConnectProgress{Stage: stage})
	}
}

// resourcePackAcquisitionBudget admits offered packs for download in offer
// order within the count and byte bounds; later packs are ignored, not fatal.
// A pack whose transfer disagrees with its offer is dropped from the handoff so
// login still succeeds, while transfers past the memory ceiling cancel the upstream dial.
// It turns gophertunnel's acquisition events into vanilla's progress figures.
type resourcePackAcquisitionBudget struct {
	proto  minecraft.Protocol
	cancel context.CancelCauseFunc

	mu          sync.Mutex
	accepted    []bool
	offered     map[string]uint64
	excluded    map[string]bool
	transferred uint64

	packs      uint32                   // admitted packs not served from the cache
	finished   uint32                   // downloads completed
	total      uint64                   // bytes of downloads begun
	received   uint64                   // bytes received
	downloads  map[string]*packDownload // pack id -> download begun
	onProgress func(ConnectProgress)
	done       bool // the dial returned; late events must not report
}

type packDownload struct {
	size, received uint64
	finished       bool
}

func newResourcePackAcquisitionBudget(proto minecraft.Protocol, cancel context.CancelCauseFunc) *resourcePackAcquisitionBudget {
	return &resourcePackAcquisitionBudget{proto: proto, cancel: cancel}
}

// observe must see ResourcePacksInfo before gophertunnel handles it: admission needs the offered sizes.
func (budget *resourcePackAcquisitionBudget) observe(header packet.Header, payload []byte) {
	if budget == nil || header.PacketID != packet.IDResourcePacksInfo {
		return
	}
	info, ok := decodeInboundPacket[*packet.ResourcePacksInfo](budget.proto, header.PacketID, payload)
	budget.admitOffer(info, ok)
}

func (budget *resourcePackAcquisitionBudget) admitOffer(info *packet.ResourcePacksInfo, decoded bool) {
	budget.mu.Lock()
	defer budget.mu.Unlock()
	budget.accepted, budget.offered = nil, map[string]uint64{}
	budget.excluded, budget.transferred = map[string]bool{}, 0
	budget.downloads = map[string]*packDownload{}
	budget.packs, budget.finished, budget.total, budget.received = 0, 0, 0, 0
	defer budget.reportLocked()
	if !decoded {
		return
	}
	budget.accepted = make([]bool, len(info.TexturePacks))
	var total uint64
	for index, pack := range info.TexturePacks {
		if int(budget.packs) == maxSelectedResourcePacks || pack.Size > maxResourcePackArchiveBytes ||
			pack.Size > maxSelectedResourcePackTotalBytes-total {
			continue
		}
		total += pack.Size
		budget.packs++
		budget.accepted[index] = true
		budget.offered[pack.UUID.String()] = pack.Size
	}
}

// event is the Dialer's ResourcePackProgress callback.
func (budget *resourcePackAcquisitionBudget) event(event minecraft.ResourcePackEvent) {
	id := event.UUID.String()
	budget.mu.Lock()
	defer budget.mu.Unlock()
	switch event.Kind {
	case minecraft.ResourcePackStarted:
		if offered, known := budget.offered[id]; !known || event.Size > offered {
			budget.excluded[id] = true // dropped from the handoff; login continues
		}
		budget.transferred = saturatingAdd(budget.transferred, event.Size)
		if budget.transferred > maxResourcePackTransferBytes {
			budget.cancel(errResourcePackTransferTooLarge)
		}
		if previous := budget.downloads[id]; previous != nil {
			budget.revertLocked(id)
		}
		budget.downloads[id] = &packDownload{size: event.Size}
		budget.total = saturatingAdd(budget.total, event.Size)
	case minecraft.ResourcePackReceived:
		download := budget.downloads[id]
		if download == nil || download.finished {
			return
		}
		download.received = saturatingAdd(download.received, event.Size)
		budget.received = saturatingAdd(budget.received, event.Size)
	case minecraft.ResourcePackFinished:
		if event.Source == minecraft.ResourcePackSourceCache {
			if _, admitted := budget.offered[id]; admitted && budget.packs > 0 {
				budget.packs--
			}
		} else if download := budget.downloads[id]; download != nil && !download.finished {
			download.finished = true
			budget.finished++
		}
	case minecraft.ResourcePackFailed:
		budget.revertLocked(id)
	}
	budget.reportLocked()
}

// revertLocked drops an abandoned download so a fallback transfer is not counted twice.
func (budget *resourcePackAcquisitionBudget) revertLocked(id string) {
	download := budget.downloads[id]
	if download == nil || download.finished {
		return
	}
	budget.total -= min(download.size, budget.total)
	budget.received -= min(download.received, budget.received)
	budget.transferred -= min(download.size, budget.transferred)
	delete(budget.downloads, id)
}

func (budget *resourcePackAcquisitionBudget) reportLocked() {
	if budget.onProgress == nil || budget.done {
		return
	}
	budget.onProgress(ConnectProgress{
		Stage:         ConnectStagePacks,
		PacksDone:     budget.finished,
		PacksTotal:    max(budget.packs, budget.finished),
		ReceivedBytes: budget.received,
		TotalBytes:    budget.total,
	})
}

// finish stops reporting once the dial has returned.
func (budget *resourcePackAcquisitionBudget) finish() {
	budget.mu.Lock()
	budget.done = true
	budget.mu.Unlock()
}

// excludes reports whether a downloaded pack must be kept out of the handoff.
func (budget *resourcePackAcquisitionBudget) excludes(pack *resource.Pack) bool {
	if budget == nil {
		return false
	}
	budget.mu.Lock()
	defer budget.mu.Unlock()
	return budget.excluded[pack.UUID().String()]
}

// admit is the Dialer's DownloadResourcePack callback.
func (budget *resourcePackAcquisitionBudget) admit(_ uuid.UUID, _ string, index, total int) bool {
	budget.mu.Lock()
	defer budget.mu.Unlock()
	return total == len(budget.accepted) && index >= 0 && index < total && budget.accepted[index]
}

// withResourcePackAcquisitionBudget routes pack admission, acquisition events and
// ResourcePacksInfo observation through budget, preserving any existing PacketFunc.
func withResourcePackAcquisitionBudget(dialer minecraft.Dialer, budget *resourcePackAcquisitionBudget) minecraft.Dialer {
	dialer.DownloadResourcePack = budget.admit
	dialer.ResourcePackProgress = budget.event
	next := dialer.PacketFunc
	dialer.PacketFunc = func(header packet.Header, payload []byte, source, destination net.Addr) {
		budget.observe(header, payload)
		if next != nil {
			next(header, payload, source, destination)
		}
	}
	return dialer
}

// decodeInboundPacket decodes a payload with the connection's own protocol;
// false means the packet cannot be trusted for budgeting.
func decodeInboundPacket[T packet.Packet](proto minecraft.Protocol, id uint32, payload []byte) (decoded T, ok bool) {
	defer func() {
		if recover() != nil {
			ok = false
		}
	}()
	factory, found := proto.Packets(false)[id]
	if !found {
		return decoded, false
	}
	pk := factory()
	buf := bytes.NewBuffer(payload)
	pk.Marshal(proto.NewReader(buf, 0, true))
	decoded, ok = pk.(T)
	return decoded, ok && buf.Len() == 0
}

// PackAdmissionError reports a typed, bounded pre-login pack failure.
type PackAdmissionError struct {
	Reason    PackAdmissionFailureReason
	PackCount int
}

type preparationCancellationError struct {
	cause error
}

func (*preparationCancellationError) Error() string {
	return "proxy: preparation cancelled by local shutdown or downstream peer"
}

func (err *preparationCancellationError) Unwrap() error { return err.cause }

func (err *PackAdmissionError) Error() string {
	return fmt.Sprintf("proxy: upstream requires %d resource pack(s), but not all of them could be acquired", err.PackCount)
}

type resourcePackOfferConnection interface {
	dialerDownstream
	ConfigureResourcePackOfferSnapshot(minecraft.ResourcePackOfferSnapshot, bool) error
	ConfigureResourcePackStack(minecraft.ResourcePackStackSnapshot, bool) error
}

// configureResourcePackOffer hands off the upstream offer and stack projected onto the admitted
// packs with the server's own required bits, as vanilla would receive them.
func configureResourcePackOffer(downstream resourcePackOfferConnection, stack *selectedResourcePackStack) error {
	if stack == nil {
		return errResourcePackStackUnavailable
	}
	if err := downstream.ConfigureResourcePackOfferSnapshot(stack.offer, stack.offer.TexturePackRequired()); err != nil {
		return err
	}
	return downstream.ConfigureResourcePackStack(stack.snapshot, stack.snapshot.Required())
}

var (
	errResourcePackStackUnavailable = errors.New("proxy: validated resource-pack stack unavailable")
)

// resourcePackStackSource is the post-negotiation seam implemented by a
// gophertunnel Dialer connection. ResourcePacks is deliberately not used here:
// it is offer/download telemetry, not the server-selected application stack.
type resourcePackStackSource interface {
	ResourcePackOffer() (minecraft.ResourcePackOfferSnapshot, bool)
	ResourcePackStack() (minecraft.ResourcePackStackSnapshot, bool)
}

// selectedResourcePackStack owns the upstream offer and stack projected onto the admitted packs
// until the prepared connection is released.
type selectedResourcePackStack struct {
	packs    []*resource.Pack // admitted content, in offer order
	required bool
	offer    minecraft.ResourcePackOfferSnapshot
	snapshot minecraft.ResourcePackStackSnapshot
}

// captureSelectedResourcePackStack admits the stack's downloaded packs within the bounds, less
// those excluded, and projects the upstream offer and stack onto them. A server that requires its
// packs gets all of them or the join is refused, as vanilla cannot join without them.
func captureSelectedResourcePackStack(upstream upstreamSession, excluded func(*resource.Pack) bool) (*selectedResourcePackStack, error) {
	source, ok := upstream.(resourcePackStackSource)
	if !ok {
		return nil, errResourcePackStackUnavailable
	}
	snapshot, ok := source.ResourcePackStack()
	if !ok {
		return nil, errResourcePackStackUnavailable
	}
	offer, ok := source.ResourcePackOffer()
	if !ok {
		return nil, errResourcePackStackUnavailable
	}
	admitted := map[string]bool{}
	for _, pack := range admitResourcePacks(snapshot.Packs(), excluded, resourcePackSize) {
		admitted[packIdentity(pack)] = true
	}
	offered, required := len(offer.TexturePacks()), offer.TexturePackRequired() || snapshot.Required()
	offer, snapshot = minecraft.ProjectResourcePacks(offer, snapshot, func(pack *resource.Pack) bool {
		return admitted[packIdentity(pack)]
	})
	if required && len(offer.TexturePacks()) != offered {
		return nil, &PackAdmissionError{Reason: PackAdmissionRequiredUnsupported, PackCount: offered}
	}
	return &selectedResourcePackStack{packs: offer.Packs(), required: required, offer: offer, snapshot: snapshot}, nil
}

func packIdentity(pack *resource.Pack) string {
	return pack.UUID().String() + "_" + pack.Version()
}

type resourcePackSizer func(*resource.Pack) (uint64, bool)

func resourcePackSize(pack *resource.Pack) (uint64, bool) {
	size := pack.Size()
	return uint64(size), size >= 0
}

// admitResourcePacks returns the packs, in stack order, that fit the count and byte bounds and
// are not excluded. Packs beyond either bound are left out rather than failing the session.
func admitResourcePacks(packs []*resource.Pack, excluded func(*resource.Pack) bool, sizeOf resourcePackSizer) []*resource.Pack {
	admitted := make([]*resource.Pack, 0, min(len(packs), maxSelectedResourcePacks))
	var total uint64
	for _, pack := range packs {
		if pack == nil || len(admitted) == maxSelectedResourcePacks || sizeOf == nil || (excluded != nil && excluded(pack)) {
			continue
		}
		size, ok := sizeOf(pack)
		if !ok || size > maxSelectedResourcePackTotalBytes-total {
			continue
		}
		total += size
		admitted = append(admitted, pack)
	}
	return admitted
}

func (stack *selectedResourcePackStack) release() {
	if stack != nil {
		stack.packs = nil
		stack.offer = minecraft.ResourcePackOfferSnapshot{}
		stack.snapshot = minecraft.ResourcePackStackSnapshot{}
	}
}

// preparedConnection owns every resource created while preparing one exact
// downstream connection. close is idempotent so cancellation and listener
// shutdown cannot double-close an upstream session or target.
type preparedConnection struct {
	upstream      upstreamSession
	releaseTarget func() error
	telemetry     *cacheBoundaryTelemetry
	logger        *slog.Logger
	packAdmission *resourcePackAdmissionTelemetry
	packStack     *selectedResourcePackStack

	closeOnce sync.Once
	closeErr  error
}

func (prepared *preparedConnection) close() error {
	return prepared.finish(true)
}

func (prepared *preparedConnection) releaseAfterRelay() error {
	return prepared.finish(false)
}

func (prepared *preparedConnection) finish(shutdownUpstream bool) error {
	if prepared == nil {
		return nil
	}
	prepared.closeOnce.Do(func() {
		prepared.closeErr = finishPreparedResources(
			shutdownUpstream,
			prepared.upstream,
			prepared.releaseTarget,
			prepared.telemetry,
			prepared.logger,
		)
		prepared.packAdmission.reportFinal()
		prepared.packStack.release()
	})
	return prepared.closeErr
}

func finishPreparedResources(
	shutdownUpstream bool,
	upstream upstreamSession,
	releaseTarget func() error,
	telemetry *cacheBoundaryTelemetry,
	logger *slog.Logger,
) error {
	var err error
	if shutdownUpstream && upstream != nil {
		err = errors.Join(err,
			callPreparedCleanup("aborting upstream", upstream.Abort),
			callPreparedCleanup("closing upstream", upstream.Close),
		)
	}
	if releaseTarget != nil {
		err = errors.Join(err, callPreparedCleanup("closing upstream target", releaseTarget))
	}
	if telemetry != nil {
		err = errors.Join(err, callPreparedCleanup("reporting upstream telemetry", func() error {
			telemetry.report(logger)
			return nil
		}))
	}
	return err
}

func callPreparedCleanup(operation string, call func() error) (err error) {
	defer func() {
		if recovered := recover(); recovered != nil {
			err = panicTypeError(operation, recovered)
		}
	}()
	return call()
}

func panicTypeError(operation string, recovered any) error {
	return fmt.Errorf("proxy: panic while %s (type %T)", operation, recovered)
}

type preparedSlot struct {
	connection *preparedConnection
	detached   chan struct{}
}

// preparedConnections retains a prepared upstream by the exact downstream
// *minecraft.Conn identity until Accept transfers ownership to the session.
type preparedConnections struct {
	tokenSource                 oauth2.TokenSource
	logger                      *slog.Logger
	upstreamClientCache         bool
	connectPrepared             func(context.Context, dialerDownstream) (*preparedConnection, error)
	resolveTarget               func(context.Context) (*resolvedUpstreamTarget, error)
	dialTarget                  func(context.Context, *resolvedUpstreamTarget, minecraft.Dialer) (upstreamSession, error)
	captureResourcePackStack    func(upstreamSession, func(*resource.Pack) bool) (*selectedResourcePackStack, error)
	resourcePackCache           minecraft.ResourcePackCache
	resourcePackAdmission       func(ResourcePackAdmissionSnapshot)
	resourcePackAdmissionUpdate func(ResourcePackAdmissionSnapshot)
	connectProgress             func(ConnectProgress)
	attempts                    atomic.Uint64

	shutdownCtx    context.Context
	shutdownCancel context.CancelFunc
	beginStopOnce  sync.Once
	finishStopOnce sync.Once
	shutdownErr    error
	prepareWG      sync.WaitGroup
	cleanupWG      sync.WaitGroup

	mu       sync.Mutex
	stopping bool
	entries  map[*minecraft.Conn]*preparedSlot
}

func newPreparedConnections(upstreamAddress string, tokenSource oauth2.TokenSource, logger *slog.Logger) *preparedConnections {
	shutdownCtx, shutdownCancel := context.WithCancel(context.Background())
	connections := &preparedConnections{
		tokenSource:    tokenSource,
		logger:         logger,
		shutdownCtx:    shutdownCtx,
		shutdownCancel: shutdownCancel,
		entries:        make(map[*minecraft.Conn]*preparedSlot),
	}
	connections.connectPrepared = connections.connect
	connections.resolveTarget = func(ctx context.Context) (*resolvedUpstreamTarget, error) {
		return resolveUpstreamTarget(ctx, upstreamAddress, tokenSource, logger)
	}
	connections.dialTarget = func(ctx context.Context, target *resolvedUpstreamTarget, dialer minecraft.Dialer) (upstreamSession, error) {
		return connectUpstream(ctx, target.address, authenticationMode(tokenSource), logger, func(ctx context.Context, address string) (upstreamSession, error) {
			return dialMinecraftUpstream(ctx, networkForAddress(target, address), address, dialer.DialContextNetwork)
		})
	}
	connections.captureResourcePackStack = captureSelectedResourcePackStack
	return connections
}

func dialMinecraftUpstream(
	ctx context.Context,
	network minecraft.Network,
	address string,
	dial func(context.Context, minecraft.Network, string) (*minecraft.Conn, error),
) (upstreamSession, error) {
	connection, err := dial(ctx, network, address)
	if connection == nil {
		return nil, err
	}
	return connection, err
}

func (connections *preparedConnections) prepare(ctx context.Context, downstream *minecraft.Conn) error {
	// The listener reads nothing while preparing, so a client that leaves (vanilla's cancel) is
	// only noticed through the stream watcher; it ends the join like the downstream closing.
	if watcher, ok := peerWatcher(downstream); ok {
		var cancelPeer context.CancelFunc
		ctx, cancelPeer = context.WithCancel(ctx)
		go func() {
			defer cancelPeer()
			select {
			case <-watcher.PeerDone():
			case <-ctx.Done():
			}
		}()
	}
	return connections.prepareConnection(ctx, downstream, downstream)
}

// peerWatcher tolerates a zero Conn, whose RemoteAddr panics.
func peerWatcher(conn *minecraft.Conn) (watcher streamnet.PeerWatcher, ok bool) {
	defer func() {
		if recover() != nil {
			watcher, ok = nil, false
		}
	}()
	watcher, ok = conn.RemoteAddr().(streamnet.PeerWatcher)
	return watcher, ok
}

func (connections *preparedConnections) prepareConnection(
	ctx context.Context,
	key *minecraft.Conn,
	downstream resourcePackOfferConnection,
) (err error) {
	connections.mu.Lock()
	if connections.stopping {
		connections.mu.Unlock()
		return context.Canceled
	}
	connections.prepareWG.Add(1)
	connections.mu.Unlock()
	defer connections.prepareWG.Done()
	defer func() {
		if err == nil || (ctx.Err() == nil && connections.shutdownCtx.Err() == nil) {
			return
		}
		err = &preparationCancellationError{cause: err}
	}()

	prepareCtx, cancel := context.WithCancel(ctx)
	stopShutdownCancellation := context.AfterFunc(connections.shutdownCtx, cancel)
	defer func() {
		stopShutdownCancellation()
		cancel()
	}()

	prepared, err := connections.connectPrepared(prepareCtx, downstream)
	if err != nil {
		return errors.Join(err, prepared.close())
	}
	owned := true
	defer func() {
		if recovered := recover(); recovered != nil {
			err = errors.Join(err, panicTypeError("configuring downstream resource-pack offer", recovered))
		}
		if owned {
			err = errors.Join(err, prepared.close())
		}
	}()
	if err = configureResourcePackOffer(downstream, prepared.packStack); err != nil {
		prepared.packAdmission.observePolicyOutcome(prepared.packStack, false)
		return err
	}
	prepared.packAdmission.observePolicyOutcome(prepared.packStack, true)
	if err = connections.store(ctx, key, prepared); err != nil {
		return err
	}
	owned = false
	return nil
}

func (connections *preparedConnections) connect(ctx context.Context, downstream dialerDownstream) (result *preparedConnection, err error) {
	telemetry := new(cacheBoundaryTelemetry)
	packAdmission := newResourcePackAdmissionTelemetry(connections.attempts.Add(1), connections.resourcePackAdmission)
	packAdmission.setUpdateCallback(connections.resourcePackAdmissionUpdate)
	var target *resolvedUpstreamTarget
	var upstream upstreamSession
	var packStack *selectedResourcePackStack
	report := func(progress ConnectProgress) {
		if connections.connectProgress != nil {
			connections.connectProgress(progress)
		}
	}
	defer report(ConnectProgress{}) // the handoff or failure ends the core's stages
	defer func() {
		if recovered := recover(); recovered != nil {
			err = panicTypeError("preparing upstream connection", recovered)
			result = nil
		}
		if result != nil {
			return
		}
		packStack.release()
		packAdmission.observeFailure(ctx)
		packAdmission.reportFinal()
		var releaseTarget func() error
		if target != nil {
			releaseTarget = target.close
		}
		err = errors.Join(err, finishPreparedResources(true, upstream, releaseTarget, telemetry, connections.logger))
	}()

	target, err = connections.resolveTarget(withConnectProgress(ctx, report))
	if err != nil {
		return nil, err
	}
	report(ConnectProgress{Stage: ConnectStageConnecting})
	dialCtx, cancelDial := context.WithCancelCause(ctx)
	budget := newResourcePackAcquisitionBudget(downstream.Proto(), cancelDial)
	budget.onProgress = report
	var cache minecraft.ResourcePackCache
	if connections.resourcePackCache != nil {
		cache = observedResourcePackCache{cache: connections.resourcePackCache, telemetry: packAdmission}
	}
	dialer := newUpstreamDialerForAdmission(downstream, connections.tokenSource, telemetry, cache, packAdmission, connections.upstreamClientCache)
	if target.xbl != nil {
		dialer.XBLClient = target.xbl
	}
	if target.playFab != nil {
		dialer.PlayFabClient = target.playFab
	}
	if target.clientData != nil {
		target.clientData(&dialer.ClientData)
	}
	dialer = withResourcePackAcquisitionBudget(dialer, budget)
	upstream, err = connections.dialTarget(dialCtx, target, dialer)
	budget.finish()
	// The dialed upstream owns its own context; releasing dialCtx now cannot
	// affect it and frees the cancellation goroutine on either outcome.
	cancelDial(nil)
	if err != nil {
		if target.realm {
			err = &realmJoinError{err: err}
		}
		return nil, err
	}
	packStack, err = connections.captureResourcePackStack(upstream, budget.excludes)
	if err != nil {
		var admission *PackAdmissionError
		if errors.As(err, &admission) {
			packAdmission.observeRejectedRequired()
		}
		return nil, err
	}
	packAdmission.observeOffer(upstream)
	result = &preparedConnection{
		upstream:      upstream,
		releaseTarget: target.close,
		telemetry:     telemetry,
		logger:        connections.logger,
		packAdmission: packAdmission,
		packStack:     packStack,
	}
	return result, nil
}

func (connections *preparedConnections) store(ctx context.Context, downstream *minecraft.Conn, prepared *preparedConnection) error {
	if downstream == nil || prepared == nil {
		return errors.New("proxy: cannot retain nil prepared connection")
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	slot := &preparedSlot{connection: prepared, detached: make(chan struct{})}
	connections.mu.Lock()
	if connections.stopping {
		connections.mu.Unlock()
		return context.Canceled
	}
	if err := ctx.Err(); err != nil {
		connections.mu.Unlock()
		return err
	}
	if _, exists := connections.entries[downstream]; exists {
		connections.mu.Unlock()
		return errors.New("proxy: duplicate prepared upstream for downstream connection")
	}
	connections.entries[downstream] = slot
	connections.mu.Unlock()

	go func() {
		select {
		case <-ctx.Done():
			_ = connections.discard(downstream, slot)
		case <-slot.detached:
		}
	}()
	return nil
}

func (connections *preparedConnections) take(downstream *minecraft.Conn) (*preparedConnection, bool) {
	connections.mu.Lock()
	slot, ok := connections.entries[downstream]
	if ok {
		delete(connections.entries, downstream)
		close(slot.detached)
	}
	connections.mu.Unlock()
	if !ok {
		return nil, false
	}
	return slot.connection, true
}

func (connections *preparedConnections) discard(downstream *minecraft.Conn, expected *preparedSlot) error {
	connections.mu.Lock()
	slot, ok := connections.entries[downstream]
	if !ok || slot != expected {
		connections.mu.Unlock()
		return nil
	}
	delete(connections.entries, downstream)
	close(slot.detached)
	connections.cleanupWG.Add(1)
	connections.mu.Unlock()
	defer connections.cleanupWG.Done()
	return slot.connection.close()
}

func (connections *preparedConnections) beginShutdown() {
	connections.beginStopOnce.Do(func() {
		connections.mu.Lock()
		connections.stopping = true
		connections.shutdownCancel()
		connections.mu.Unlock()
	})
}

func (connections *preparedConnections) finishShutdown() error {
	connections.beginShutdown()
	connections.finishStopOnce.Do(func() {
		connections.prepareWG.Wait()

		connections.mu.Lock()
		entries := make([]*preparedConnection, 0, len(connections.entries))
		for downstream, slot := range connections.entries {
			delete(connections.entries, downstream)
			close(slot.detached)
			entries = append(entries, slot.connection)
		}
		connections.mu.Unlock()
		for _, prepared := range entries {
			connections.shutdownErr = errors.Join(connections.shutdownErr, prepared.close())
		}
		connections.cleanupWG.Wait()
	})
	return connections.shutdownErr
}

func (connections *preparedConnections) shutdown() error {
	connections.beginShutdown()
	return connections.finishShutdown()
}

func servePreparedConnection(ctx context.Context, downstream downstreamSession, prepared *preparedConnection) (err error) {
	relayCompleted := false
	defer func() {
		if relayCompleted {
			err = errors.Join(err, prepared.releaseAfterRelay())
			return
		}
		err = errors.Join(err, shutdownSession(downstream), prepared.close())
	}()
	err = relayPacketsWithCacheTelemetry(ctx, downstream, prepared.upstream, prepared.telemetry)
	relayCompleted = true
	return err
}
