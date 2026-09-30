package localworld

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
)

// DefaultBDSImage is the community image that downloads the requested official BDS build inside the
// container, applies the EULA acknowledgement and maps settings from environment variables. It is used
// instead of bind-mounting our own zip because it also supplies the Linux runtime libraries and a signal
// handler that saves the world on `docker stop`; nothing of Mojang's is bundled by us either way.
// Needs pinning to a digest at release time.
const DefaultBDSImage = "itzg/minecraft-bedrock-server:latest"

const (
	containerStartTimeout = 10 * time.Minute // first start pulls the image and downloads BDS
	containerStopSeconds  = 25               // inside the manager's 30s stop budget
)

func (r BDSRunner) dockerBin() string {
	if r.Docker != "" {
		return r.Docker
	}
	return "docker"
}

func (r BDSRunner) image() string {
	if r.Image != "" {
		return r.Image
	}
	return DefaultBDSImage
}

func (r BDSRunner) dockerCmd(ctx context.Context, args ...string) *exec.Cmd {
	cmd := exec.CommandContext(ctx, r.dockerBin(), args...)
	cmd.Env = append(os.Environ(), r.Env...)
	return cmd
}

func containerName(worldID string) string { return "cinnabar-bds-" + worldID }

// ensureImage pulls the image once; the pull shows as a download in the setup status.
func (r BDSRunner) ensureImage(ctx context.Context) error {
	if err := r.dockerCmd(ctx, "image", "inspect", r.image()).Run(); err == nil {
		return nil
	}
	p := r.Provisioner
	p.setOp(SetupDownloading, "", 0, 0)
	out, err := r.dockerCmd(ctx, "pull", "--platform", "linux/amd64", r.image()).CombinedOutput()
	if err != nil {
		p.log().Error("docker pull failed", "output", strings.TrimSpace(string(out)))
		return p.fail(fmt.Errorf("localworld: pull server image: %w", err))
	}
	p.setOp("", "", 0, 0)
	return nil
}

// containerArgs builds `docker run`: loopback-only UDP mapping, the world folder bind-mounted into the
// server's worlds directory, and the EULA acknowledged (callers check acceptance first).
func containerArgs(spec StartSpec, image, version, dataDir string, hostPort, maxPlayers int) []string {
	w := spec.World
	levelType := "DEFAULT"
	if w.Generator == GeneratorFlat {
		levelType = "FLAT"
	}
	view := clampInt(orDefault(spec.Options.ViewDistance, defaultBDSView), 5, 32)
	args := []string{
		"run", "--rm", "--name", containerName(w.ID), "--platform", "linux/amd64",
		"-p", fmt.Sprintf("127.0.0.1:%d:19132/udp", hostPort),
		"-v", dataDir + ":/data",
		"-v", filepath.Join(spec.Dir, "db") + ":/data/worlds/" + w.ID,
	}
	for _, kv := range [][2]string{
		{"EULA", "TRUE"}, {"VERSION", version}, {"SERVER_NAME", sanitizeProperty(w.Name)},
		{"GAMEMODE", w.GameMode}, {"DIFFICULTY", w.Difficulty}, {"ALLOW_CHEATS", "false"},
		{"MAX_PLAYERS", strconv.Itoa(maxPlayers)}, {"ONLINE_MODE", "false"}, {"ALLOW_LIST", "false"},
		{"LEVEL_NAME", w.ID}, {"LEVEL_SEED", strconv.FormatInt(w.Seed, 10)}, {"LEVEL_TYPE", levelType},
		{"VIEW_DISTANCE", strconv.Itoa(view)}, {"TICK_DISTANCE", strconv.Itoa(clampInt(view, 4, 12))},
		{"PLAYER_IDLE_TIMEOUT", "0"},
	} {
		args = append(args, "-e", kv[0]+"="+kv[1])
	}
	return append(args, image)
}

func orDefault(v, fallback int) int {
	if v <= 0 {
		return fallback
	}
	return v
}

func (r BDSRunner) startContainer(ctx context.Context, spec StartSpec) (Instance, error) {
	p := r.Provisioner
	if !p.eulaAccepted() {
		return nil, ErrEULARequired
	}
	log := r.Log
	if log == nil {
		log = slog.Default()
	}
	timeout := r.StartTimeout
	if timeout <= 0 {
		timeout = containerStartTimeout
	}
	maxPlayers := r.MaxPlayers
	if maxPlayers <= 0 {
		maxPlayers = 1
	}
	version, err := p.exactVersion(ctx)
	if err != nil {
		return nil, err
	}
	if err := r.ensureImage(ctx); err != nil {
		return nil, err
	}
	dataDir := filepath.Join(p.Root, "container-data")
	worldDir := filepath.Join(spec.Dir, "db")
	for _, dir := range []string{dataDir, worldDir} {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return nil, fmt.Errorf("localworld: create %s: %w", filepath.Base(dir), err)
		}
	}
	address, err := freeLoopbackAddress()
	if err != nil {
		return nil, err
	}
	name := containerName(spec.World.ID)
	_ = r.dockerCmd(ctx, "rm", "-f", name).Run() // a leftover from a crashed core
	cmd := r.dockerCmd(context.Background(), containerArgs(spec, r.image(), version, dataDir, portOf(address), maxPlayers)...)
	inst, err := launch(ctx, launchSpec{
		cmd: cmd, address: address, log: log.With("component", "bds-container", "world", spec.World.ID), timeout: timeout,
		ready: func(line string) bool { return strings.Contains(line, bdsReadyMarker) },
	})
	if err != nil {
		_ = r.dockerCmd(context.Background(), "rm", "-f", name).Run()
		return nil, err
	}
	wrapped := &containerInstance{Instance: inst, runner: r, name: name}
	go func() {
		<-inst.Done()
		wrapped.cleanupOnce()
	}()
	return wrapped, nil
}

// containerInstance stops the container gracefully (BDS saves on SIGTERM) and always removes it.
type containerInstance struct {
	Instance
	runner BDSRunner
	name   string
	once   sync.Once
}

func (c *containerInstance) cleanupOnce() {
	c.once.Do(func() { _ = c.runner.dockerCmd(context.Background(), "rm", "-f", c.name).Run() })
}

func (c *containerInstance) Stop(ctx context.Context) error {
	_ = c.runner.dockerCmd(ctx, "stop", "-t", strconv.Itoa(containerStopSeconds), c.name).Run()
	err := c.Instance.Stop(ctx)
	c.cleanupOnce()
	return err
}

// CanPause is false for the same reason as native BDS.
func (c *containerInstance) CanPause() bool { return false }
