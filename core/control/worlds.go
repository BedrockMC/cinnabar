package control

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net"

	"github.com/hashimthearab/rust-mcbe/core/localworld"
)

const (
	methodWorldList   = "world_list.v1"
	methodWorldCreate = "world_create.v1"
	methodWorldRename = "world_rename.v1"
	methodWorldDelete = "world_delete.v1"
	methodWorldOpen   = "world_open.v1"
	methodWorldClose  = "world_close.v1"
	methodWorldPause  = "world_pause.v1"
	methodWorldStatus = "world_status.v1"

	// maxListedWorlds keeps a world_list response inside MaxFrameLen.
	maxListedWorlds = 200

	codeWorldFailed   = -32000
	codeWorldNotFound = -32010
	codeWorldBusy     = -32011
)

// Worlds is the local-world service behind the world_* methods; *localworld.Manager implements it.
type Worlds interface {
	List() ([]localworld.World, error)
	Create(localworld.Spec) (localworld.World, error)
	Rename(id, name string) (localworld.World, error)
	Delete(id string) error
	Open(id string) error
	Close() error
	SetPaused(paused bool) error
	Status() localworld.Status
}

var worldMethods = map[string]struct{}{
	methodWorldList: {}, methodWorldCreate: {}, methodWorldRename: {}, methodWorldDelete: {},
	methodWorldOpen: {}, methodWorldClose: {}, methodWorldPause: {}, methodWorldStatus: {},
}

func isWorldMethod(method string) bool {
	_, ok := worldMethods[method]
	return ok
}

// WorldResultV1 is the result of every world_* method; unused members are omitted.
type WorldResultV1 struct {
	SchemaVersion uint32             `json:"schema_version"`
	Worlds        []localworld.World `json:"worlds,omitempty"`
	World         *localworld.World  `json:"world,omitempty"`
	Status        *localworld.Status `json:"status,omitempty"`
}

type worldResponse struct {
	JSONRPC string         `json:"jsonrpc"`
	ID      uint64         `json:"id"`
	Result  *WorldResultV1 `json:"result,omitempty"`
	Error   *responseError `json:"error,omitempty"`
}

func decodeParams(raw json.RawMessage, into any) bool {
	if len(raw) == 0 {
		return false
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	return decoder.Decode(into) == nil && decoder.Decode(new(any)) == io.EOF
}

func (server *Server) serveWorld(conn net.Conn, id uint64, method string, raw json.RawMessage) error {
	fail := func(code int, message string) error {
		return server.writeResponse(conn, worldResponse{JSONRPC: "2.0", ID: id, Error: &responseError{Code: code, Message: message}})
	}
	invalid := func() error { return fail(-32602, "Invalid params") }
	worlds := server.worlds
	result := &WorldResultV1{SchemaVersion: 1}
	var err error
	switch method {
	case methodWorldList, methodWorldClose, methodWorldStatus:
		if len(raw) != 0 {
			return invalid()
		}
	}
	switch method {
	case methodWorldList:
		result.Worlds, err = worlds.List()
		if len(result.Worlds) > maxListedWorlds {
			result.Worlds = result.Worlds[:maxListedWorlds]
		}
	case methodWorldCreate:
		var spec localworld.Spec
		if !decodeParams(raw, &spec) {
			return invalid()
		}
		var world localworld.World
		if world, err = worlds.Create(spec); err == nil {
			result.World = &world
		}
	case methodWorldRename:
		var params struct {
			ID   *string `json:"id"`
			Name *string `json:"name"`
		}
		if !decodeParams(raw, &params) || params.ID == nil || params.Name == nil {
			return invalid()
		}
		var world localworld.World
		if world, err = worlds.Rename(*params.ID, *params.Name); err == nil {
			result.World = &world
		}
	case methodWorldDelete, methodWorldOpen:
		var params struct {
			ID *string `json:"id"`
		}
		if !decodeParams(raw, &params) || params.ID == nil {
			return invalid()
		}
		if method == methodWorldDelete {
			err = worlds.Delete(*params.ID)
		} else {
			err = worlds.Open(*params.ID)
		}
	case methodWorldClose:
		err = worlds.Close()
	case methodWorldPause:
		var params struct {
			Paused *bool `json:"paused"`
		}
		if !decodeParams(raw, &params) || params.Paused == nil {
			return invalid()
		}
		err = worlds.SetPaused(*params.Paused)
	}
	if err != nil {
		return fail(worldErrorCode(err), worldErrorMessage(err))
	}
	if method == methodWorldOpen || method == methodWorldClose || method == methodWorldPause || method == methodWorldStatus {
		status := worlds.Status()
		result.Status = &status
	}
	return server.writeResponse(conn, worldResponse{JSONRPC: "2.0", ID: id, Result: result})
}

func worldErrorCode(err error) int {
	switch {
	case errors.Is(err, localworld.ErrNotFound):
		return codeWorldNotFound
	case errors.Is(err, localworld.ErrBusy), errors.Is(err, localworld.ErrInUse), errors.Is(err, localworld.ErrNotOpen):
		return codeWorldBusy
	case errors.Is(err, localworld.ErrInvalid):
		return -32602
	}
	return codeWorldFailed
}

// worldErrorMessage exposes only sentinel-class messages; other errors may carry local paths.
func worldErrorMessage(err error) string {
	for _, known := range []error{localworld.ErrNotFound, localworld.ErrBusy, localworld.ErrInUse, localworld.ErrNotOpen} {
		if errors.Is(err, known) {
			return known.Error()
		}
	}
	if errors.Is(err, localworld.ErrInvalid) {
		return err.Error()
	}
	return "world operation failed"
}
