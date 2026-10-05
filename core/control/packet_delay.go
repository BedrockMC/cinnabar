package control

import (
	"bytes"
	"encoding/json"
	"io"
	"net"

	"github.com/hashimthearab/rust-mcbe/core/proxy"
)

const methodPacketDelay = "packet_delay.v1"

func (server *Server) SetPacketDelay(delay *proxy.PacketDelay) {
	server.mu.Lock()
	server.packetDelay = delay
	server.mu.Unlock()
}

func (server *Server) servePacketDelay(conn net.Conn, id uint64, raw json.RawMessage) error {
	server.mu.Lock()
	delay := server.packetDelay
	server.mu.Unlock()
	if delay == nil {
		return server.writeResponse(conn, response{JSONRPC: "2.0", ID: id, Error: &responseError{Code: -32601, Message: "Method not found"}})
	}
	var params struct {
		DelayMS *uint32 `json:"delay_ms"`
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&params); err != nil || decoder.Decode(new(any)) != io.EOF || params.DelayMS == nil || delay.Set(*params.DelayMS) != nil {
		return server.writeResponse(conn, response{JSONRPC: "2.0", ID: id, Error: &responseError{Code: -32602, Message: "Invalid params"}})
	}
	return server.writeResponse(conn, response{JSONRPC: "2.0", ID: id, Result: struct {
		DelayMS       uint32 `json:"delay_ms"`
		LeaseMS       int64  `json:"lease_ms"`
		SchemaVersion uint32 `json:"schema_version"`
	}{*params.DelayMS, proxy.PacketDelayLease.Milliseconds(), 1}})
}
