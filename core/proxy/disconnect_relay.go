package proxy

import (
	"errors"

	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

// upstreamRelayDisconnect records which connection produced a disconnect. A
// reverse-direction write can observe the upstream close before its reader does.
type upstreamRelayDisconnect struct {
	cause error
	value packet.Disconnect
}

func (e *upstreamRelayDisconnect) Error() string { return e.cause.Error() }
func (e *upstreamRelayDisconnect) Unwrap() error { return e.cause }

func attributeRelayError(err error, fromUpstream bool) error {
	var disconnect *minecraft.DisconnectPacketError
	if fromUpstream && errors.As(err, &disconnect) && disconnect != nil {
		return &upstreamRelayDisconnect{cause: err, value: *disconnect.Packet()}
	}
	return err
}

type packetDisconnecter interface {
	DisconnectPacket(packet.Disconnect) error
}

// relayPreLoginDisconnect tells a downstream that has not spawned yet why its join failed: a
// server's own disconnect packet found anywhere in err, else vanilla's lang key for the failure.
func relayPreLoginDisconnect(downstream packetDisconnecter, err error) {
	var disconnect *minecraft.DisconnectPacketError
	if errors.As(err, &disconnect) && disconnect != nil {
		_ = callWithoutPanic(func() error { return downstream.DisconnectPacket(*disconnect.Packet()) })
		return
	}
	var cancelled *preparationCancellationError
	if err == nil || errors.As(err, &cancelled) {
		return
	}
	_ = callWithoutPanic(func() error { return downstream.DisconnectPacket(packet.Disconnect{Message: joinFailureKey(err)}) })
}

func joinFailureKey(err error) string {
	var realm *realmJoinError
	switch {
	case errors.Is(err, errResourcePackTransferTooLarge):
		return "disconnectionScreen.resourcePack"
	case errors.As(err, &realm):
		return "disconnectionScreen.cantConnectToRealm"
	default:
		return "disconnectionScreen.cantConnect"
	}
}
