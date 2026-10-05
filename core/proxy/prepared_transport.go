package proxy

import (
	"context"
	"errors"
	"net"
	"reflect"
	"sync/atomic"

	"github.com/sandertv/gophertunnel/minecraft"
)

// preparedTransport overlaps identity-independent transport setup with authentication.
// It returns the original connection so optional packet transport capabilities survive.
type preparedTransport struct {
	minecraft.Network
	address   string
	cancel    context.CancelFunc
	done      chan struct{}
	claimed   atomic.Bool
	handedOff atomic.Bool
	disposed  atomic.Bool
	conn      net.Conn
	err       error
}

func newPreparedTransport(ctx context.Context, network minecraft.Network, address string) *preparedTransport {
	ctx, cancel := context.WithCancel(ctx)
	prepared := &preparedTransport{Network: network, address: address, cancel: cancel, done: make(chan struct{})}
	go func() {
		defer close(prepared.done)
		defer func() {
			if recovered := recover(); recovered != nil {
				prepared.err = panicTypeError("preparing upstream transport", recovered)
			}
		}()
		prepared.conn, prepared.err = network.DialContext(ctx, address)
		prepared.conn = usableTransport(prepared.conn)
		if prepared.conn == nil && prepared.err == nil {
			prepared.err = net.ErrClosed
		}
	}()
	return prepared
}

func (prepared *preparedTransport) DialContext(ctx context.Context, address string) (net.Conn, error) {
	if address != prepared.address {
		return nil, errors.New("proxy: prepared transport target changed")
	}
	if !prepared.claimed.CompareAndSwap(false, true) {
		return nil, errors.New("proxy: prepared transport already claimed")
	}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-prepared.done:
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		if prepared.err != nil {
			return nil, prepared.err
		}
		if connection, ok := prepared.conn.(interface{ Context() context.Context }); ok && connection.Context().Err() != nil {
			// The upstream closes an idle connection once its login deadline passes, which slow
			// authentication can outlast; finish closes the expired one and login takes a fresh dial.
			return prepared.Network.DialContext(ctx, address)
		}
		prepared.handedOff.Store(true)
		return prepared.conn, nil
	}
}

// Network implementations may return an interface containing a nil connection.
func usableTransport(conn net.Conn) net.Conn {
	if conn == nil {
		return nil
	}
	value := reflect.ValueOf(conn)
	switch value.Kind() {
	case reflect.Chan, reflect.Func, reflect.Interface, reflect.Map, reflect.Pointer, reflect.Slice:
		if value.IsNil() {
			return nil
		}
	}
	return conn
}

// finish cancels unfinished setup and closes any transport the login did not retain.
func (prepared *preparedTransport) finish(retained bool) {
	prepared.cancel()
	<-prepared.done
	if (!retained || !prepared.handedOff.Load()) && prepared.disposed.CompareAndSwap(false, true) && prepared.conn != nil {
		_ = prepared.conn.Close()
	}
}

func dialWithPreparedTransport(
	ctx context.Context,
	network minecraft.Network,
	address string,
	dial func(context.Context, minecraft.Network, string) (*minecraft.Conn, error),
) (connection *minecraft.Conn, err error) {
	// NetherNet proves possession during transport setup and must authenticate first.
	switch network.(type) {
	case minecraft.RakNet, *minecraft.RakNet:
		prepared := newPreparedTransport(ctx, network, address)
		defer func() { prepared.finish(connection != nil && err == nil) }()
		connection, err = dial(ctx, prepared, address)
		if err != nil && prepared.handedOff.Load() && ctx.Err() == nil && stalePreLogin(err) {
			// The upstream can expire the early connection while its shutdown is still deferred, so
			// login fails on a live-looking transport; retry once on a fresh one.
			prepared.finish(false)
			return dial(ctx, network, address)
		}
		return connection, err
	default:
		return dial(ctx, network, address)
	}
}

// stalePreLogin reports a login that failed on a closed transport rather than a server answer.
func stalePreLogin(err error) bool {
	var transfer *minecraft.TransferError
	var disconnect minecraft.DisconnectError
	if errors.As(err, &transfer) || errors.As(err, &disconnect) {
		return false
	}
	return errors.Is(err, context.Canceled) || errors.Is(err, net.ErrClosed)
}
