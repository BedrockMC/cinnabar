package proxy

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"strconv"
	"time"

	"github.com/df-mc/go-nethernet/endpoint"
	"github.com/hashimthearab/rust-mcbe/core/localworld"
	"github.com/sandertv/gophertunnel/minecraft"
)

// LocalTargetFunc supplies the local server's address and transport together.
type LocalTargetFunc func(context.Context) (localworld.ConnectionTarget, bool, error)

// withLocalTarget never routes a selected local world through online discovery or authentication.
func withLocalTarget(local LocalTargetFunc, online func(context.Context) (*resolvedUpstreamTarget, error)) func(context.Context) (*resolvedUpstreamTarget, error) {
	if local == nil {
		return online
	}
	return func(ctx context.Context) (*resolvedUpstreamTarget, error) {
		target, ok, err := local(ctx)
		if err != nil {
			return nil, err
		}
		if !ok {
			return online(ctx)
		}
		switch target.Transport {
		case localworld.TransportRakNet:
			return &resolvedUpstreamTarget{address: target.Address, network: minecraft.RakNet{}}, nil
		case localworld.TransportNetherNetLAN:
			return resolveLocalLANTarget(ctx, target)
		case localworld.TransportNetherNetHTTP:
			host, port, err := net.SplitHostPort(target.Address)
			ip := net.ParseIP(host)
			portNumber, portErr := strconv.ParseUint(port, 10, 16)
			isLoopback := ip != nil && ip.IsLoopback()
			if err != nil || !isLoopback {
				return nil, fmt.Errorf("local NetherNet target is not a loopback address: %q", target.Address)
			}
			if portErr != nil || portNumber == 0 {
				return nil, fmt.Errorf("local NetherNet target has invalid port: %q", target.Address)
			}
			client := endpoint.ClientConfig{HTTPClient: &http.Client{
				Timeout: 30 * time.Second,
				// Local BDS never redirects: do not let a local target forward
				// status requests or SDP offers to another origin.
				CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
			}}.New()
			return &resolvedUpstreamTarget{
				address: "http://" + target.Address,
				network: localNetherNetNetwork{NetherNet: minecraft.NetherNet{Signaling: client}, status: client},
			}, nil
		default:
			return nil, fmt.Errorf("unsupported local transport %q", target.Transport)
		}
	}
}

// Local BDS uses Mojang's GET status and full-ICE HTTP SDP exchange, without Xbox signaling.
type localNetherNetNetwork struct {
	minecraft.NetherNet
	status *endpoint.Client
}

func (n localNetherNetNetwork) PingContext(ctx context.Context, address string) ([]byte, error) {
	return n.status.PingContext(ctx, address)
}
