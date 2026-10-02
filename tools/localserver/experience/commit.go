package experience

import (
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"unicode"

	"github.com/df-mc/dragonfly/server/block/cube"
	"github.com/df-mc/dragonfly/server/player"
	"github.com/df-mc/dragonfly/server/world"
)

// formattingPrefix starts a Minecraft formatting code, which a tell may not hold.
const formattingPrefix = '§'

// errStale discards a result whose snapshot no longer matches the world, or whose actor has left
// it. It is an expected race, not a fault.
var errStale = errors.New("stale result")

// errInvalid discards a result holding an op that the runtime should never have committed. It is
// a bug in the runtime or the adapter, so the Experience is not blamed.
var errInvalid = errors.New("invalid result")

// teller sends a tell to a player.
type teller interface {
	tell(p world.Entity, text string)
}

// messageTeller tells a player with a chat message.
type messageTeller struct{}

func (messageTeller) tell(p world.Entity, text string) {
	if pl, ok := p.(*player.Player); ok {
		pl.Message(text)
	}
}

// commit applies ops, the committed result of ev's callback on snap, in a fresh task of the
// event's world, unless the snapshot went stale or an op is invalid; then it applies nothing.
func (h *Host) commit(ctx context.Context, d *dispatcher, ev event, snap snapshot, ops []Op) {
	var result error
	task := ev.w.Do(func(tx *world.Tx) { result = h.apply(tx, d.id, ev, snap, ops) })
	if err := task.Wait(ctx); err != nil {
		task.Cancel()
		if ctx.Err() == nil {
			h.log.Warn("commit failed", "experience", d.id, "error", err)
		}
		return
	}
	switch {
	case errors.Is(result, errStale):
		h.log.Info("stale result discarded", "experience", d.id, "reason", result)
	case result != nil:
		h.log.Error("invalid result discarded", "experience", d.id, "error", result)
	}
}

// apply checks snap against the world in tx and validates every op before it applies any: first
// the block and data ops in their order, then the tells.
func (h *Host) apply(tx *world.Tx, exp string, ev event, snap snapshot, ops []Op) error {
	actor, err := h.current(tx, exp, ev, snap)
	if err != nil {
		return err
	}
	data, err := h.validate(exp, ev, snap, ops)
	if err != nil {
		return err
	}
	for i, op := range ops {
		switch {
		case op.SetBlock != nil:
			pos := op.SetBlock.Pos.cube()
			if op.SetBlock.ID == airID {
				tx.SetBlock(pos, nil, nil)
				h.store.Remove(exp, ev.dim.storeKey(pos))
				continue
			}
			b, _ := h.reg.Lookup(op.SetBlock.ID)
			tx.SetBlock(pos, b, nil)
			h.store.Place(exp, ev.dim.storeKey(pos))
		case op.SetBlockData != nil:
			key := ev.dim.storeKey(op.SetBlockData.Pos.cube())
			if err := h.store.SetData(exp, key, data[i], op.SetBlockData.Data != nil); err != nil {
				// validate checked ownership and the quota, so the store disagrees with it.
				h.log.Error("validated data write failed", "experience", exp, "op", i, "error", err)
			}
		}
	}
	for _, op := range ops {
		if op.Tell != nil {
			h.tell.tell(actor, op.Tell.Text)
		}
	}
	return nil
}

// current checks that every snapshot cell still has its loaded state, block id and store token,
// and that the event's actor, if any, is a player in the world. It returns the actor's entity.
func (h *Host) current(tx *world.Tx, exp string, ev event, snap snapshot) (world.Entity, error) {
	for _, was := range snap.cells {
		now := h.cellState(tx, exp, ev.dim, was.pos)
		if now.loaded != was.loaded || now.id != was.id || now.hasToken != was.hasToken ||
			now.token != was.token {
			return nil, fmt.Errorf("%w: the cell at %v changed", errStale, was.pos)
		}
	}
	if ev.actor == nil {
		return nil, nil
	}
	for p := range tx.Players() {
		if p.H() == ev.actor {
			return p, nil
		}
	}
	return nil, fmt.Errorf("%w: actor %s is not in the world", errStale, ev.actor.UUID())
}

// simCell is a writable snapshot cell as the ops before the current one leave it.
type simCell struct {
	id      string
	owned   bool
	dataLen uint64
}

// validate checks every op against the snapshot as the ops before it change it, by the rules
// the runtime enforced: writes stay in the anchor's chunk column on loaded snapshot cells, set
// air or an own block over air or an own block, write data only to an own block within the size
// limit and the quota, and tell only the actor, within the tell limits. It returns the decoded
// data of each data op, by op index.
func (h *Host) validate(exp string, ev event, snap snapshot, ops []Op) ([][]byte, error) {
	if len(ops) > maxStagedOps {
		return nil, fmt.Errorf("%w: %d ops, at most %d", errInvalid, len(ops), maxStagedOps)
	}
	column := func(pos cube.Pos) [2]int { return [2]int{pos[0] >> 4, pos[2] >> 4} }
	cells := make(map[cube.Pos]*simCell, len(snap.cells))
	for _, c := range snap.cells {
		if c.loaded && column(c.pos) == column(ev.anchor) {
			cells[c.pos] = &simCell{id: c.id, owned: c.owned, dataLen: c.dataLen}
		}
	}
	writable := func(i int, pos BlockPos) (*simCell, error) {
		if c, ok := cells[pos.cube()]; ok {
			return c, nil
		}
		return nil, fmt.Errorf("%w: op %d writes %v, outside the write scope", errInvalid, i, pos)
	}
	var actorID string
	if ev.actor != nil {
		actorID = ev.actor.UUID().String()
	}
	used := dataQuota - h.store.Budget(exp)
	data := make([][]byte, len(ops))
	tells := 0
	for i, op := range ops {
		switch {
		case op.SetBlock != nil:
			c, err := writable(i, op.SetBlock.Pos)
			if err != nil {
				return nil, err
			}
			if c.id != airID && !c.owned {
				return nil, fmt.Errorf("%w: op %d sets a block over %s, which is not its own",
					errInvalid, i, c.id)
			}
			id := op.SetBlock.ID
			if b, ok := h.reg.Lookup(id); id != airID && (!ok || b.t.exp != exp) {
				return nil, fmt.Errorf("%w: op %d sets %q, which is not its own block", errInvalid, i, id)
			}
			used -= c.dataLen
			*c = simCell{id: id, owned: id != airID}
		case op.SetBlockData != nil:
			c, err := writable(i, op.SetBlockData.Pos)
			if err != nil {
				return nil, err
			}
			if !c.owned {
				return nil, fmt.Errorf("%w: op %d writes data to %s, which is not its own block",
					errInvalid, i, c.id)
			}
			if hexData := op.SetBlockData.Data; hexData != nil {
				if data[i], err = hex.DecodeString(*hexData); err != nil {
					return nil, fmt.Errorf("%w: op %d data: %v", errInvalid, i, err)
				}
			}
			n := uint64(len(data[i]))
			if n > maxBlockDataBytes {
				return nil, fmt.Errorf("%w: op %d writes %d bytes of data, at most %d",
					errInvalid, i, n, maxBlockDataBytes)
			}
			if used = used - c.dataLen + n; used > dataQuota {
				return nil, fmt.Errorf("%w: op %d exceeds the data quota", errInvalid, i)
			}
			c.dataLen = n
		case op.Tell != nil:
			text := op.Tell.Text
			tells++
			switch {
			case ev.actor == nil || op.Tell.Player != actorID:
				return nil, fmt.Errorf("%w: op %d tells %s, who is not the actor", errInvalid, i, op.Tell.Player)
			case tells > maxTells:
				return nil, fmt.Errorf("%w: more than %d tells", errInvalid, maxTells)
			case len(text) > maxTellBytes:
				return nil, fmt.Errorf("%w: op %d tells %d bytes, at most %d", errInvalid, i, len(text), maxTellBytes)
			case strings.ContainsFunc(text, func(r rune) bool { return unicode.IsControl(r) || r == formattingPrefix }):
				return nil, fmt.Errorf("%w: op %d tells a control or formatting character", errInvalid, i)
			}
		default:
			return nil, fmt.Errorf("%w: op %d is empty", errInvalid, i)
		}
	}
	return data, nil
}
