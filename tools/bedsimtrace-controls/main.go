// Command bedsimtrace-controls emits only the processed primary-control
// contract at exact BedSim main 34d11dc5. It is not a kinematic trace.
package main

import (
	"encoding/json"
	"io"
	"os"

	"github.com/df-mc/dragonfly/server/block"
	"github.com/df-mc/dragonfly/server/block/cube"
	"github.com/df-mc/dragonfly/server/world"
	"github.com/go-gl/mathgl/mgl32"
	"github.com/oomph-ac/bedsim"
)

type emptyWorld struct{}

func (emptyWorld) Block(cube.Pos) world.Block                { return block.Air{} }
func (emptyWorld) BlockCollisions(cube.Pos) []cube.BBox32    { return nil }
func (emptyWorld) GetNearbyBBoxes(cube.BBox32) []cube.BBox32 { return nil }
func (emptyWorld) IsChunkLoaded(int32, int32) bool           { return true }

type controlsInput struct {
	Strafe      float32  `json:"strafe"`
	Forward     float32  `json:"forward"`
	Yaw         float32  `json:"yaw_degrees"`
	Jumping     bool     `json:"jumping"`
	JumpPressed bool     `json:"jump_pressed"`
	Sprinting   bool     `json:"sprinting"`
	Sneaking    bool     `json:"sneaking"`
	Raw         bool     `json:"move_vector_is_raw"`
	Consuming   bool     `json:"using_consumable"`
	Modifier    *float32 `json:"item_use_movement_modifier,omitempty"`
}
type record struct {
	Name      string        `json:"name"`
	Input     controlsInput `json:"input"`
	Processed [2]float32    `json:"processed"`
}

func cases() []record {
	base := controlsInput{Strafe: 0.25, Forward: -0.5}
	makeCase := func(name string, raw, sneak, consuming bool, modifier *float32) record {
		input := base
		input.Raw, input.Sneaking, input.Consuming, input.Modifier = raw, sneak, consuming, modifier
		return record{Name: name, Input: input}
	}
	zero, half, one := float32(0), float32(0.5), float32(1)
	records := []record{
		makeCase("processed_walk", false, false, false, nil),
		makeCase("processed_sneak", false, true, false, nil),
		makeCase("raw_walk", true, false, false, nil),
		makeCase("raw_sneak", true, true, false, nil),
		makeCase("processed_consuming", false, false, true, nil),
		makeCase("raw_consuming", true, false, true, nil),
		makeCase("raw_explicit_zero", true, false, true, &zero),
		makeCase("raw_explicit_half", true, false, true, &half),
		makeCase("raw_explicit_one", true, false, true, &one),
		makeCase("raw_composed_sneak", true, true, true, &half),
	}
	bounded := makeCase("raw_component_bounds", true, true, false, nil)
	bounded.Input.Strafe, bounded.Input.Forward = 2, -2
	nonbinary := makeCase("raw_nonbinary_sneak", true, true, false, nil)
	nonbinary.Input.Strafe, nonbinary.Input.Forward = 0.7, -0.7
	unequal := makeCase("raw_nonbinary_unequal_sneak", true, true, false, nil)
	unequal.Input.Strafe, unequal.Input.Forward = 0.1, 0.9
	sevenTenths := float32(0.7)
	composed := makeCase("raw_nonbinary_item_pose", true, true, true, &sevenTenths)
	composed.Input.Strafe, composed.Input.Forward = 0.7, -0.9
	return append(records, bounded, nonbinary, unequal, composed)
}

func emit(w io.Writer) error {
	encoder := json.NewEncoder(w)
	simulator := bedsim.Simulator{World: emptyWorld{}}
	for _, record := range cases() {
		state := bedsim.MovementState{Size: mgl32.Vec3{0.6, 1.8, 0.6}, MovementSpeed: 0.1, DefaultMovementSpeed: 0.1, AirSpeed: 0.02, Alive: true, Ready: true, TicksSinceTeleport: 1, TicksSinceKnockback: 1}
		input := record.Input
		result := simulator.Simulate(&state, bedsim.InputState{MoveVector: mgl32.Vec2{input.Strafe, input.Forward}, MoveVectorIsRaw: input.Raw, SneakDown: input.Sneaking, UsingConsumable: input.Consuming, ItemUseMovementModifier: input.Modifier})
		record.Processed = [2]float32(result.InputMoveVector)
		if err := encoder.Encode(record); err != nil {
			return err
		}
	}
	return nil
}

func main() {
	if err := emit(os.Stdout); err != nil {
		panic(err)
	}
}
