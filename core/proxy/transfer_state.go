package proxy

import (
	"errors"
	"log/slog"
	"net"
	"strconv"
	"strings"
	"sync"

	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

// TransferTarget is a server-directed transfer destination.
type TransferTarget struct {
	Host string
	Port uint16
}

// TransferState holds the upstream that the next local client connection dials
// after a server-directed transfer. The zero value is ready to use.
type TransferState struct {
	mu   sync.Mutex
	next string
	// OnTransfer, when set, is called after each recorded transfer; it must not block.
	OnTransfer func(TransferTarget)
}

// Record makes target the next upstream. Unusable targets return an error and change nothing.
func (s *TransferState) Record(target TransferTarget) error {
	address, err := transferAddress(target.Host, target.Port)
	if err != nil {
		return err
	}
	s.mu.Lock()
	s.next = address
	callback := s.OnTransfer
	s.mu.Unlock()
	if callback != nil {
		host, port, _ := net.SplitHostPort(address)
		parsed, _ := strconv.ParseUint(port, 10, 16)
		callback(TransferTarget{Host: host, Port: uint16(parsed)})
	}
	return nil
}

// Upstream returns the recorded transfer address, or initial if none was recorded.
func (s *TransferState) Upstream(initial string) string {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.next == "" {
		return initial
	}
	return s.next
}

// transferAddress joins a transfer host and port into a dialable address.
func transferAddress(host string, port uint16) (string, error) {
	host = strings.TrimSpace(host)
	if strings.HasPrefix(host, "[") && strings.HasSuffix(host, "]") {
		host = strings.TrimSpace(host[1 : len(host)-1])
	}
	if host == "" {
		return "", errors.New("proxy: invalid transfer: empty address")
	}
	if port == 0 {
		return "", errors.New("proxy: invalid transfer: zero port")
	}
	return net.JoinHostPort(host, strconv.Itoa(int(port))), nil
}

// transferObservingSession records Transfer packets read from upstream and still relays them.
type transferObservingSession struct {
	upstreamSession
	state  *TransferState
	logger *slog.Logger
}

func observeTransfers(upstream upstreamSession, state *TransferState, logger *slog.Logger) upstreamSession {
	if state == nil {
		return upstream
	}
	return &transferObservingSession{upstreamSession: upstream, state: state, logger: logger}
}

func (s *transferObservingSession) ReadBatch() ([]packet.Packet, error) {
	batch, err := s.upstreamSession.ReadBatch()
	for _, value := range batch {
		transfer, ok := value.(*packet.Transfer)
		if !ok {
			continue
		}
		if recordErr := s.state.Record(TransferTarget{Host: transfer.Address, Port: transfer.Port}); recordErr != nil && s.logger != nil {
			s.logger.Warn("ignoring unusable server transfer", "error", recordErr)
		}
	}
	return batch, err
}
