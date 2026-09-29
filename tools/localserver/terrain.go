package main

import (
	"math"

	"github.com/df-mc/dragonfly/server/block"
	"github.com/df-mc/dragonfly/server/block/cube"
	"github.com/df-mc/dragonfly/server/world"
	"github.com/df-mc/dragonfly/server/world/biome"
	"github.com/df-mc/dragonfly/server/world/chunk"
	"github.com/df-mc/dragonfly/server/world/generator"
)

const (
	seaLevel      = 62
	baseHeight    = 66
	heightSpread  = 26
	noiseScale    = 96.0 // blocks per lowest-frequency noise cell
	noiseOctaves  = 4
	dirtDepth     = 3
	surfaceMargin = 1 // blocks above sea level that stay sand
)

// normal is a seeded value-noise heightmap: an approximation, not vanilla terrain.
type normal struct {
	seed                                          int64
	air, bedrock, stone, dirt, grass, sand, water uint32
	plains                                        uint32
}

func newNormal(seed int64) normal {
	return normal{
		seed:    seed,
		air:     world.BlockRuntimeID(block.Air{}),
		bedrock: world.BlockRuntimeID(block.Bedrock{}),
		stone:   world.BlockRuntimeID(block.Stone{}),
		dirt:    world.BlockRuntimeID(block.Dirt{}),
		grass:   world.BlockRuntimeID(block.Grass{}),
		sand:    world.BlockRuntimeID(block.Sand{}),
		water:   world.BlockRuntimeID(block.Water{Still: true, Depth: 8}),
		plains:  uint32(biome.Plains{}.EncodeBiome()),
	}
}

func flatFor(dim world.Dimension) world.Generator {
	switch dim {
	case world.Nether:
		return generator.NewFlat(biome.NetherWastes{}, []world.Block{block.Netherrack{}, block.Netherrack{}, block.Netherrack{}, block.Bedrock{}})
	case world.End:
		return generator.NewFlat(biome.End{}, []world.Block{block.EndStone{}, block.EndStone{}, block.EndStone{}, block.Bedrock{}})
	}
	return generator.NewFlat(biome.Plains{}, []world.Block{block.Grass{}, block.Dirt{}, block.Dirt{}, block.Bedrock{}})
}

// mix is a splitmix64 finaliser.
func mix(v uint64) uint64 {
	v += 0x9e3779b97f4a7c15
	v = (v ^ (v >> 30)) * 0xbf58476d1ce4e5b9
	v = (v ^ (v >> 27)) * 0x94d049bb133111eb
	return v ^ (v >> 31)
}

// lattice returns a deterministic value in [0,1) for an integer noise-lattice point.
func (n normal) lattice(octave int, x, z int64) float64 {
	h := mix(uint64(n.seed) ^ mix(uint64(octave)+1))
	h = mix(h ^ uint64(x))
	h = mix(h ^ uint64(z)*0x2545f4914f6cdd1d)
	return float64(h>>11) / (1 << 53)
}

func smooth(t float64) float64 { return t * t * (3 - 2*t) }

func lerp(a, b, t float64) float64 { return a + (b-a)*t }

func (n normal) valueNoise(octave int, x, z float64) float64 {
	fx, fz := math.Floor(x), math.Floor(z)
	tx, tz := smooth(x-fx), smooth(z-fz)
	ix, iz := int64(fx), int64(fz)
	top := lerp(n.lattice(octave, ix, iz), n.lattice(octave, ix+1, iz), tx)
	bottom := lerp(n.lattice(octave, ix, iz+1), n.lattice(octave, ix+1, iz+1), tx)
	return lerp(top, bottom, tz)
}

// height returns the surface Y of a block column.
func (n normal) height(x, z int) int {
	sum, amplitude, total, scale := 0.0, 1.0, 0.0, noiseScale
	for octave := range noiseOctaves {
		sum += n.valueNoise(octave, float64(x)/scale, float64(z)/scale) * amplitude
		total += amplitude
		amplitude /= 2
		scale /= 2
	}
	return baseHeight + int(math.Round((sum/total-0.5)*2*heightSpread))
}

// GenerateChunk implements world.Generator.
func (n normal) GenerateChunk(pos world.ChunkPos, c *chunk.Chunk) {
	min, max := int16(c.Range().Min()), int16(c.Range().Max())
	for x := range uint8(16) {
		for z := range uint8(16) {
			h := n.height(int(pos[0])*16+int(x), int(pos[1])*16+int(z))
			for y := min; y <= max; y++ {
				if rid := n.blockAt(int(y), int(min), h); rid != n.air {
					c.SetBlock(x, y, z, 0, rid)
				}
				c.SetBiome(x, y, z, n.plains)
			}
		}
	}
}

// blockAt returns the block for height y in a column whose surface is h.
func (n normal) blockAt(y, min, h int) uint32 {
	switch {
	case y == min:
		return n.bedrock
	case y < h-dirtDepth:
		return n.stone
	case y < h:
		if h <= seaLevel+surfaceMargin {
			return n.sand
		}
		return n.dirt
	case y == h:
		if h <= seaLevel+surfaceMargin {
			return n.sand
		}
		return n.grass
	case y <= seaLevel:
		return n.water
	}
	return n.air
}

// DefaultSpawn implements world.Generator; it spawns on dry land near the origin.
func (n normal) DefaultSpawn(dim world.Dimension) cube.Pos {
	for r := 0; r < 512; r += 8 {
		for _, p := range [][2]int{{r, 0}, {-r, 0}, {0, r}, {0, -r}} {
			if h := n.height(p[0], p[1]); h > seaLevel+surfaceMargin {
				return cube.Pos{p[0], h + 1, p[1]}
			}
		}
	}
	return cube.Pos{0, seaLevel + 2, 0}
}
