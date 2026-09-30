package proxy

import (
	"context"
	"crypto/ecdsa"
	"errors"
	"fmt"
	"log/slog"
	"net"
	"strconv"
	"strings"
	"time"

	"github.com/df-mc/go-nethernet"
	"github.com/df-mc/go-playfab/v2"
	"github.com/df-mc/go-xsapi/v2"
	"github.com/google/uuid"
	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/auth"
	"github.com/sandertv/gophertunnel/minecraft/p2p"
	"github.com/sandertv/gophertunnel/minecraft/protocol/login"
	"github.com/sandertv/gophertunnel/minecraft/realms"
	"github.com/sandertv/gophertunnel/minecraft/service"
	"golang.org/x/oauth2"
)

const (
	friendTargetPrefix = "friend_xuid/"
	realmTargetPrefix  = "realm_id/"
	realmCodePrefix    = "realm/"
	// netherNetTargetPrefix names a raw NetherNet ID with its signaling, as nethernet/jsonrpc/<id>.
	netherNetTargetPrefix = "nethernet/"
)

// LocalTargetFunc returns the address of a local game server; ok is false when none is selected.
type LocalTargetFunc func(ctx context.Context) (address string, ok bool, err error)

// withLocalTarget routes to the local server when one is selected, else to the online resolver.
func withLocalTarget(local LocalTargetFunc, online func(context.Context) (*resolvedUpstreamTarget, error)) func(context.Context) (*resolvedUpstreamTarget, error) {
	if local == nil {
		return online
	}
	return func(ctx context.Context) (*resolvedUpstreamTarget, error) {
		address, ok, err := local(ctx)
		if err != nil {
			return nil, err
		}
		if !ok {
			return online(ctx)
		}
		return &resolvedUpstreamTarget{address: address, network: minecraft.RakNet{}}, nil
	}
}

type resolvedUpstreamTarget struct {
	address    string
	network    minecraft.Network
	clientData func(*login.ClientData) // applies a joined session's login fields
	xbl        *xsapi.Client
	playFab    *playfab.Client
	friend     interface{ Close() error }
	realm      bool // vanilla words a failed Realm join as its own
}

// realmJoinError marks a failure while joining a Realm.
type realmJoinError struct{ err error }

func (e *realmJoinError) Error() string { return e.err.Error() }
func (e *realmJoinError) Unwrap() error { return e.err }

func (target *resolvedUpstreamTarget) close() error {
	if target == nil {
		return nil
	}
	var joined error
	if target.friend != nil {
		joined = errors.Join(joined, target.friend.Close())
	}
	if target.playFab != nil {
		joined = errors.Join(joined, target.playFab.Close())
	}
	if target.xbl != nil {
		joined = errors.Join(joined, target.xbl.Close())
	}
	return joined
}

func resolveUpstreamTarget(ctx context.Context, address string, src oauth2.TokenSource, logger *slog.Logger) (*resolvedUpstreamTarget, error) {
	address = strings.TrimSpace(address)
	if address == "" {
		return nil, errors.New("upstream target is empty")
	}
	if src == nil {
		if isStableTarget(address) {
			return nil, errors.New("authenticated target requires a Microsoft session")
		}
		return &resolvedUpstreamTarget{address: address, network: minecraft.RakNet{}}, nil
	}

	resolveContext, cancel := context.WithTimeout(ctx, 45*time.Second)
	defer cancel()
	switch {
	case strings.HasPrefix(strings.ToLower(address), friendTargetPrefix):
		return resolveFriendTarget(resolveContext, address, src, logger)
	case strings.HasPrefix(strings.ToLower(address), realmTargetPrefix),
		strings.HasPrefix(strings.ToLower(address), realmCodePrefix):
		return resolveRealmTarget(resolveContext, address, src, logger)
	case strings.HasPrefix(strings.ToLower(address), netherNetTargetPrefix):
		return resolveRawNetherNetTarget(resolveContext, address, src, logger)
	case isRawNetherNetAddress(address):
		return nil, fmt.Errorf("NetherNet target %q needs its signaling: use %sjsonrpc/<id> or %swebsocket/<id>", address, netherNetTargetPrefix, netherNetTargetPrefix)
	default:
		return &resolvedUpstreamTarget{address: address, network: minecraft.RakNet{}}, nil
	}
}

func resolveRealmTarget(ctx context.Context, address string, src oauth2.TokenSource, logger *slog.Logger) (*resolvedUpstreamTarget, error) {
	reportConnectStage(ctx, ConnectStageRealm)
	target, err := lookupRealmTarget(ctx, address, src, logger)
	if err != nil {
		return nil, &realmJoinError{err: err}
	}
	target.realm = true
	return target, nil
}

func lookupRealmTarget(ctx context.Context, address string, src oauth2.TokenSource, logger *slog.Logger) (*resolvedUpstreamTarget, error) {
	client := realms.NewClient(src, nil)
	var realmAddress realms.RealmAddress
	var err error
	if strings.HasPrefix(strings.ToLower(address), realmTargetPrefix) {
		id, parseErr := strconv.Atoi(strings.TrimSpace(address[len(realmTargetPrefix):]))
		if parseErr != nil || id <= 0 {
			return nil, fmt.Errorf("invalid realm target %q", address)
		}
		realmAddress, err = client.RealmAddress(ctx, id)
	} else {
		code := strings.TrimSpace(address[len(realmCodePrefix):])
		realm, lookupErr := client.Realm(ctx, code)
		if lookupErr == nil {
			realmAddress, err = realm.Address(ctx)
		} else {
			err = lookupErr
		}
	}
	if err != nil {
		return nil, fmt.Errorf("resolve realm %q: %w", address, err)
	}
	if strings.TrimSpace(realmAddress.Address) == "" {
		return nil, fmt.Errorf("resolve realm %q: empty address", address)
	}
	protocol := realms.ParseNetworkProtocol(string(realmAddress.NetworkProtocol))
	if protocol == realms.NetworkProtocolDefault || protocol == "" {
		return &resolvedUpstreamTarget{address: realmAddress.Address, network: minecraft.RakNet{}}, nil
	}
	connectionType, ok := realmConnectionType(protocol)
	if !ok {
		return nil, fmt.Errorf("realm %q uses unsupported network protocol %q", address, realmAddress.NetworkProtocol)
	}
	return newNetherNetTarget(ctx, realmAddress.Address, connectionType, src, nil, logger)
}

func resolveFriendTarget(ctx context.Context, address string, src oauth2.TokenSource, logger *slog.Logger) (*resolvedUpstreamTarget, error) {
	xuid := strings.TrimSpace(address[len(friendTargetPrefix):])
	if separator := strings.IndexByte(xuid, ':'); separator >= 0 {
		xuid = xuid[:separator]
	}
	if xuid == "" {
		return nil, fmt.Errorf("invalid friend target %q", address)
	}
	xbl, err := newXSAPIClient(ctx, src)
	if err != nil {
		return nil, err
	}
	closeXBLOnError := true
	defer func() {
		if closeXBLOnError {
			_ = xbl.Close()
		}
	}()
	worlds, err := p2p.NewClient(xbl).Worlds(ctx)
	if err != nil {
		return nil, fmt.Errorf("request friend worlds: %w", err)
	}
	world := selectFriendWorld(worlds, xuid)
	if world == nil {
		return nil, fmt.Errorf("friend world %q is no longer joinable", xuid)
	}
	session, err := world.Join(ctx)
	if err != nil {
		return nil, fmt.Errorf("join friend world %q: %w", xuid, err)
	}
	joined, err := p2p.ClientTargetFromSession(session)
	if err != nil {
		_ = session.Close()
		return nil, fmt.Errorf("join friend world %q: %w", xuid, err)
	}
	target, err := newNetherNetTarget(ctx, joined.DialAddress(), joined.ConnectionType(), src, xbl, logger)
	if err != nil {
		_ = joined.Close()
		return nil, err
	}
	target.clientData = joined.ApplyClientData
	target.friend = joined
	closeXBLOnError = false
	return target, nil
}

// selectFriendWorld prefers a friends-joinable world of the owner and falls back to an
// invite-only one the account can already see; nil when the owner hosts nothing joinable.
func selectFriendWorld(worlds []p2p.World, ownerXUID string) *p2p.World {
	var inviteOnly *p2p.World
	for index := range worlds {
		candidate := &worlds[index]
		if candidate.OwnerID != ownerXUID {
			continue
		}
		switch candidate.Joinability {
		case p2p.JoinabilityFriends:
			return candidate
		case p2p.JoinabilityInviteOnly:
			if inviteOnly == nil {
				inviteOnly = candidate
			}
		}
	}
	return inviteOnly
}

func resolveRawNetherNetTarget(ctx context.Context, address string, src oauth2.TokenSource, logger *slog.Logger) (*resolvedUpstreamTarget, error) {
	id, connectionType, err := parseNetherNetTarget(address)
	if err != nil {
		return nil, err
	}
	return newNetherNetTarget(ctx, id, connectionType, src, nil, logger)
}

// parseNetherNetTarget splits nethernet/<signaling>/<id>; the signaling is never inferred from the ID.
func parseNetherNetTarget(address string) (string, int, error) {
	signaling, id, _ := strings.Cut(address[len(netherNetTargetPrefix):], "/")
	connectionType, ok := map[string]int{
		"jsonrpc":   p2p.ConnectionTypeSignalingOverJSONRPC,
		"websocket": p2p.ConnectionTypeSignalingOverWebSocket,
	}[strings.ToLower(signaling)]
	if !ok || !isRawNetherNetAddress(id) {
		return "", 0, fmt.Errorf("invalid NetherNet target %q: want %sjsonrpc/<id> or %swebsocket/<id>", address, netherNetTargetPrefix, netherNetTargetPrefix)
	}
	return id, connectionType, nil
}

func newNetherNetTarget(ctx context.Context, address string, connectionType int, src oauth2.TokenSource, xbl *xsapi.Client, logger *slog.Logger) (*resolvedUpstreamTarget, error) {
	if xbl == nil {
		var err error
		xbl, err = newXSAPIClient(ctx, src)
		if err != nil {
			return nil, err
		}
	}
	serviceSource, playFab, err := newServiceTokenSource(ctx, xbl)
	if err != nil {
		_ = xbl.Close()
		return nil, err
	}
	network := newScopedNetherNetNetwork(serviceSource, connectionType, logger)
	return &resolvedUpstreamTarget{
		address: address,
		network: network,
		xbl:     xbl,
		playFab: playFab,
	}, nil
}

func newXSAPIClient(ctx context.Context, src oauth2.TokenSource) (*xsapi.Client, error) {
	client, err := xsapi.ClientConfig{RTAMode: xsapi.RTALazy}.New(ctx, xsapiTokenSource(src))
	if err != nil {
		return nil, fmt.Errorf("login to Xbox Live: %w", err)
	}
	return client, nil
}

func xsapiTokenSource(src oauth2.TokenSource) xsapi.TokenSource {
	if cached, ok := src.(xsapi.TokenSource); ok {
		return cached
	}
	return auth.AndroidConfig.New(src, nil)
}

func newServiceTokenSource(ctx context.Context, xbl *xsapi.Client) (service.TokenSource, *playfab.Client, error) {
	discovery, err := service.Default(ctx)
	if err != nil {
		return nil, nil, fmt.Errorf("discover Minecraft services: %w", err)
	}
	env := new(service.AuthorizationEnvironment)
	if err := discovery.Environment(env); err != nil {
		return nil, nil, fmt.Errorf("resolve Minecraft services: %w", err)
	}
	playFab, err := playfab.LoginWithXbox(ctx, env.PlayFabTitleID, xbl, playfab.ClientConfig{CreateAccount: true})
	if err != nil {
		return nil, nil, fmt.Errorf("login to PlayFab: %w", err)
	}
	return env.TokenSource(playFab, service.TokenConfig{}), playFab, nil
}

func realmConnectionType(protocol realms.NetworkProtocol) (int, bool) {
	switch realms.ParseNetworkProtocol(string(protocol)) {
	case realms.NetworkProtocolNetherNet:
		return p2p.ConnectionTypeSignalingOverWebSocket, true
	case realms.NetworkProtocolNetherNetJSONRPC:
		return p2p.ConnectionTypeSignalingOverJSONRPC, true
	default:
		return 0, false
	}
}

func isStableTarget(address string) bool {
	lower := strings.ToLower(strings.TrimSpace(address))
	return strings.HasPrefix(lower, friendTargetPrefix) || strings.HasPrefix(lower, realmTargetPrefix) || strings.HasPrefix(lower, realmCodePrefix)
}

func isRawNetherNetAddress(address string) bool {
	if _, err := strconv.ParseUint(address, 10, 64); err == nil {
		return true
	}
	return uuid.Validate(address) == nil
}

// scopedNetherNetNetwork dials through gophertunnel's NetherNet so authenticated dials present
// the Login's multiplayer token and key as the SDP identity, as vanilla's MinecraftIdentityAssertion does.
type scopedNetherNetNetwork struct {
	signal minecraft.DialSignalingFunc // fresh signaling per dial; the transport owns and closes it
	logger *slog.Logger
}

func newScopedNetherNetNetwork(serviceSource service.TokenSource, connectionType int, logger *slog.Logger) scopedNetherNetNetwork {
	signal := func(ctx context.Context, _ string) (minecraft.SignalingConn, error) {
		conn, err := p2p.DialClientSignaling(ctx, connectionType, serviceSource, p2p.ClientSignalingOptions{Log: logger})
		if err != nil {
			return nil, fmt.Errorf("establish NetherNet signaling: %w", err)
		}
		return conn, nil
	}
	return scopedNetherNetNetwork{signal: signal, logger: logger}
}

// transport accepts identityless answers like vanilla's ClientNegotiator::onRemoteAnswer, while
// go-nethernet still verifies a server identity that is present.
func (network scopedNetherNetNetwork) transport() minecraft.NetherNet {
	return minecraft.NetherNet{
		DialSignaling: network.signal,
		Dialer:        nethernet.Dialer{Log: network.logger, AllowIdentitylessServer: true},
	}
}

func (network scopedNetherNetNetwork) DialContext(ctx context.Context, address string) (net.Conn, error) {
	return wrapNetherNetDial(network.transport().DialContext(ctx, address))
}

// DialContextIdentityProvider is used by minecraft.Dialer for authenticated dials.
func (network scopedNetherNetNetwork) DialContextIdentityProvider(ctx context.Context, address, token string, key *ecdsa.PrivateKey, identityProvider string) (net.Conn, error) {
	return wrapNetherNetDial(network.transport().DialContextIdentityProvider(ctx, address, token, key, identityProvider))
}

func wrapNetherNetDial(conn net.Conn, err error) (net.Conn, error) {
	if err != nil {
		return nil, fmt.Errorf("dial NetherNet: %w", err)
	}
	return conn, nil
}

func (scopedNetherNetNetwork) PingContext(context.Context, string) ([]byte, error) {
	return nil, errors.New("NetherNet ping is unsupported")
}

func (scopedNetherNetNetwork) Listen(string) (minecraft.NetworkListener, error) {
	return nil, errors.New("NetherNet listen is unsupported")
}
