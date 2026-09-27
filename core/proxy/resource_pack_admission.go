package proxy

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"log/slog"
	"math"
	"net"
	"sync"
	"sync/atomic"
	"time"

	"github.com/google/uuid"
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
	// Cinnabar safety bound on the download phase, below the client's login timeout.
	maxResourcePackAcquisitionTime = 60 * time.Second
)

var (
	errResourcePackAcquisitionTimeout = errors.New("proxy: resource-pack acquisition exceeded its time bound")
	errResourcePackArchiveTooLarge    = errors.New("proxy: resource-pack archive exceeds its size bound")
)

// resourcePackAcquisitionBudget admits offered packs for download in offer
// order until the count or byte bound is reached; later packs are ignored, not
// fatal. A transfer that outgrows its bounds cancels the upstream dial.
type resourcePackAcquisitionBudget struct {
	proto  minecraft.Protocol
	cancel context.CancelCauseFunc
	limit  time.Duration

	mu       sync.Mutex
	accepted []bool
	timer    *time.Timer
}

func newResourcePackAcquisitionBudget(proto minecraft.Protocol, cancel context.CancelCauseFunc) *resourcePackAcquisitionBudget {
	return &resourcePackAcquisitionBudget{proto: proto, cancel: cancel, limit: maxResourcePackAcquisitionTime}
}

// observe must see every inbound packet before gophertunnel handles it.
func (budget *resourcePackAcquisitionBudget) observe(header packet.Header, payload []byte) {
	if budget == nil {
		return
	}
	switch header.PacketID {
	case packet.IDResourcePacksInfo:
		info, ok := decodeInboundPacket[*packet.ResourcePacksInfo](budget.proto, header.PacketID, payload)
		budget.admitOffer(info, ok)
	case packet.IDResourcePackDataInfo:
		info, ok := decodeInboundPacket[*packet.ResourcePackDataInfo](budget.proto, header.PacketID, payload)
		if ok && info.Size > maxResourcePackArchiveBytes {
			budget.cancel(errResourcePackArchiveTooLarge)
		}
	case packet.IDResourcePackStack, packet.IDStartGame:
		budget.stop()
	}
}

func (budget *resourcePackAcquisitionBudget) admitOffer(info *packet.ResourcePacksInfo, decoded bool) {
	budget.mu.Lock()
	defer budget.mu.Unlock()
	budget.accepted = nil
	if !decoded {
		return
	}
	budget.accepted = make([]bool, len(info.TexturePacks))
	var total uint64
	admitted := 0
	for index, pack := range info.TexturePacks {
		if admitted == maxSelectedResourcePacks || pack.Size > maxResourcePackArchiveBytes ||
			pack.Size > maxSelectedResourcePackTotalBytes-total {
			continue
		}
		total += pack.Size
		admitted++
		budget.accepted[index] = true
	}
	if budget.timer != nil {
		budget.timer.Stop()
		budget.timer = nil
	}
	if admitted != 0 {
		budget.timer = time.AfterFunc(budget.limit, func() { budget.cancel(errResourcePackAcquisitionTimeout) })
	}
}

// admit is the Dialer's DownloadResourcePack callback.
func (budget *resourcePackAcquisitionBudget) admit(_ uuid.UUID, _ string, index, total int) bool {
	budget.mu.Lock()
	defer budget.mu.Unlock()
	return total == len(budget.accepted) && index >= 0 && index < total && budget.accepted[index]
}

func (budget *resourcePackAcquisitionBudget) stop() {
	budget.mu.Lock()
	defer budget.mu.Unlock()
	if budget.timer != nil {
		budget.timer.Stop()
		budget.timer = nil
	}
}

// withResourcePackAcquisitionBudget routes pack admission and inbound packet
// observation through budget, preserving any existing PacketFunc.
func withResourcePackAcquisitionBudget(dialer minecraft.Dialer, budget *resourcePackAcquisitionBudget) minecraft.Dialer {
	dialer.DownloadResourcePack = budget.admit
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
	return fmt.Sprintf("proxy: upstream requires %d resource pack(s), but pack application is unavailable", err.PackCount)
}

type resourcePackOfferConnection interface {
	dialerDownstream
	ConfigureResourcePackOffer([]*resource.Pack, bool) error
	ConfigureResourcePackStack(minecraft.ResourcePackStackSnapshot, bool) error
}

// configureResourcePackOffer offers only the acquired selected archives, in
// stack order, and replays the exact upstream stack. The private hop is always
// optional so an unavailable pack never blocks login; the client ignores stack
// entries it was not offered.
func configureResourcePackOffer(downstream resourcePackOfferConnection, stack *selectedResourcePackStack) error {
	if stack == nil {
		return errResourcePackStackUnavailable
	}
	if err := downstream.ConfigureResourcePackOffer(stack.packs, false); err != nil {
		return err
	}
	return downstream.ConfigureResourcePackStack(stack.snapshot, false)
}

var (
	errResourcePackStackUnavailable = errors.New("proxy: validated resource-pack stack unavailable")
	errResourcePackStackInvalid     = errors.New("proxy: validated resource-pack stack is invalid")
)

// resourcePackStackSource is the post-negotiation seam implemented by a
// gophertunnel Dialer connection. ResourcePacks is deliberately not used here:
// it is offer/download telemetry, not the server-selected application stack.
type resourcePackStackSource interface {
	ResourcePackOffer() (minecraft.ResourcePackOfferSnapshot, bool)
	ResourcePackStack() (minecraft.ResourcePackStackSnapshot, bool)
}

// selectedResourcePackStack owns immutable pack clones in exact application
// order until the prepared connection is released.
type selectedResourcePackStack struct {
	packs    []*resource.Pack
	required bool
	snapshot minecraft.ResourcePackStackSnapshot
}

func captureSelectedResourcePackStack(upstream upstreamSession) (*selectedResourcePackStack, error) {
	source, ok := upstream.(resourcePackStackSource)
	if !ok {
		return nil, errResourcePackStackUnavailable
	}
	snapshot, ok := source.ResourcePackStack()
	if !ok {
		return nil, errResourcePackStackUnavailable
	}
	stack, err := newSelectedResourcePackStack(snapshot.Packs(), snapshot.Required(), resourcePackSize)
	if err != nil {
		return nil, err
	}
	entries := snapshot.Entries()
	packIndex := 0
	for _, entry := range entries {
		pack := entry.Pack()
		if pack == nil {
			continue
		}
		if packIndex >= len(stack.packs) || pack.UUID() != stack.packs[packIndex].UUID() || pack.Version() != stack.packs[packIndex].Version() {
			stack.release()
			return nil, errResourcePackStackInvalid
		}
		packIndex++
	}
	if packIndex != len(stack.packs) {
		stack.release()
		return nil, errResourcePackStackInvalid
	}
	stack.snapshot = snapshot
	return stack, nil
}

type resourcePackSizer func(*resource.Pack) (uint64, bool)

func resourcePackSize(pack *resource.Pack) (uint64, bool) {
	size := pack.Size()
	return uint64(size), size >= 0
}

func newSelectedResourcePackStack(packs []*resource.Pack, required bool, sizeOf resourcePackSizer) (*selectedResourcePackStack, error) {
	if len(packs) > maxSelectedResourcePacks || sizeOf == nil {
		return nil, errResourcePackStackInvalid
	}
	owned := make([]*resource.Pack, len(packs))
	var total uint64
	for index, pack := range packs {
		if pack == nil {
			return nil, errResourcePackStackInvalid
		}
		size, ok := sizeOf(pack)
		if !ok || size > math.MaxUint64-total || total+size > maxSelectedResourcePackTotalBytes {
			return nil, errResourcePackStackInvalid
		}
		total += size
		owned[index] = pack.Clone()
	}
	return &selectedResourcePackStack{packs: owned, required: required}, nil
}

func (stack *selectedResourcePackStack) release() {
	if stack != nil {
		stack.packs = nil
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
	captureResourcePackStack    func(upstreamSession) (*selectedResourcePackStack, error)
	resourcePackCache           minecraft.ResourcePackCache
	resourcePackAdmission       func(ResourcePackAdmissionSnapshot)
	resourcePackAdmissionUpdate func(ResourcePackAdmissionSnapshot)
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
			return dialMinecraftUpstream(ctx, target.network, address, dialer.DialContextNetwork)
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
	return connections.prepareConnection(ctx, downstream, downstream)
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

	target, err = connections.resolveTarget(ctx)
	if err != nil {
		return nil, err
	}
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
	if target.clientData.nonce != "" {
		dialer.ClientData.Nonce = target.clientData.nonce
	}
	dialCtx, cancelDial := context.WithCancelCause(ctx)
	budget := newResourcePackAcquisitionBudget(dialer.Protocol, cancelDial)
	dialer = withResourcePackAcquisitionBudget(dialer, budget)
	upstream, err = connections.dialTarget(dialCtx, target, dialer)
	budget.stop()
	if err != nil {
		cancelDial(nil)
		return nil, err
	}
	packStack, err = connections.captureResourcePackStack(upstream)
	if err != nil {
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
	if err := spawnBarrier(ctx, downstream, prepared.upstream); err != nil {
		return err
	}
	err = relayPacketsWithCacheTelemetry(ctx, downstream, prepared.upstream, prepared.telemetry)
	relayCompleted = true
	return err
}
