package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"path/filepath"

	"github.com/df-mc/dragonfly/server"
	"github.com/df-mc/dragonfly/server/world"
)

const maxPlayers = 4 // one local player plus a reconnect overlapping its predecessor

// settings are the per-world options the core passes on the command line.
type settings struct {
	dir, addr, name, gameMode, diff string
	// experiences is the directory of server Experience artifacts, empty for none; runtime is the
	// experience-runtime binary that runs them.
	experiences, runtime string
}

func parseSettings(args []string, stderr io.Writer) (settings, error) {
	var s settings
	flags := flag.NewFlagSet("bedrock-local-server", flag.ContinueOnError)
	flags.SetOutput(stderr)
	flags.StringVar(&s.dir, "dir", "", "world data directory")
	flags.StringVar(&s.addr, "addr", "", "loopback UDP listen address")
	flags.StringVar(&s.name, "name", "World", "world display name")
	flags.StringVar(&s.gameMode, "game-mode", "survival", "survival, creative or adventure")
	flags.StringVar(&s.diff, "difficulty", "normal", "peaceful, easy, normal or hard")
	flags.StringVar(&s.experiences, "experiences", "", "directory of server Experience artifacts")
	flags.StringVar(&s.runtime, "experience-runtime", "", "experience-runtime binary; required with -experiences")
	if err := flags.Parse(args); err != nil {
		return settings{}, err
	}
	if s.dir == "" || s.addr == "" {
		return settings{}, errors.New("-dir and -addr are required")
	}
	if s.experiences != "" && s.runtime == "" {
		return settings{}, errors.New("-experience-runtime is required with -experiences")
	}
	if _, err := s.worldGameMode(); err != nil {
		return settings{}, err
	}
	if _, err := s.worldDifficulty(); err != nil {
		return settings{}, err
	}
	return s, nil
}

func (s settings) worldGameMode() (world.GameMode, error) {
	switch s.gameMode {
	case "survival":
		return world.GameModeSurvival, nil
	case "creative":
		return world.GameModeCreative, nil
	case "adventure":
		return world.GameModeAdventure, nil
	}
	return nil, fmt.Errorf("unknown game mode %q", s.gameMode)
}

func (s settings) worldDifficulty() (world.Difficulty, error) {
	switch s.diff {
	case "peaceful":
		return world.DifficultyPeaceful, nil
	case "easy":
		return world.DifficultyEasy, nil
	case "normal":
		return world.DifficultyNormal, nil
	case "hard":
		return world.DifficultyHard, nil
	}
	return nil, fmt.Errorf("unknown difficulty %q", s.diff)
}

// userConfig is an offline, loopback-only server whose data lives under s.dir.
func (s settings) userConfig() server.UserConfig {
	uc := server.DefaultConfig()
	uc.Network.Address = s.addr
	uc.Network.Transport = []string{"raknet"}
	uc.Server.Name = s.name
	uc.Server.AuthEnabled = false
	uc.Server.DisableJoinQuitMessages = true
	uc.World.SaveData = true
	uc.World.Folder = filepath.Join(s.dir, "db")
	uc.Players.SaveData = true
	uc.Players.Folder = filepath.Join(s.dir, "players")
	uc.Players.MaxCount = maxPlayers
	uc.Resources.Folder = filepath.Join(s.dir, "resources")
	return uc
}

// applyTo sets the world's gameplay defaults; they are re-applied on every start so the stored settings win over level.dat.
func (s settings) applyTo(worlds ...*world.World) {
	mode, _ := s.worldGameMode()
	difficulty, _ := s.worldDifficulty()
	for _, w := range worlds {
		w.SetDefaultGameMode(mode)
		w.SetDifficulty(difficulty)
	}
}
