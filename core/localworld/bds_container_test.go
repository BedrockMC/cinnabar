package localworld

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func fakeDockerEnv(t *testing.T, extra ...string) (env []string, logPath string) {
	t.Helper()
	logPath = filepath.Join(t.TempDir(), "docker.log")
	return append([]string{helperEnv + "=docker", dockerLogEnv + "=" + logPath}, extra...), logPath
}

func macProvisioner(t *testing.T) *Provisioner {
	t.Helper()
	p := &Provisioner{Root: filepath.Join(t.TempDir(), "bds"), goos: "darwin", goarch: "arm64", Version: "1.26.52.3"}
	p.SetRuntime(RuntimeInfo{Kind: RuntimeContainer, Reason: "container"})
	return p
}

func TestContainerRunnerLifecycleAndArguments(t *testing.T) {
	env, logPath := fakeDockerEnv(t)
	p := macProvisioner(t)
	if st := p.Status(); st.State != SetupEULARequired || st.Runtime != RuntimeContainer {
		t.Fatalf("status = %+v", st)
	}
	runner := BDSRunner{Provisioner: p, Docker: os.Args[0], Env: env, StartTimeout: 20 * time.Second}
	spec := testSpec()
	spec.Dir = t.TempDir()
	if _, err := runner.Start(context.Background(), spec); err != ErrEULARequired {
		t.Fatalf("before EULA: %v", err)
	}
	if err := p.AcceptEULA(); err != nil {
		t.Fatal(err)
	}
	if st := p.Status(); st.State != SetupReady {
		t.Fatalf("status = %+v", st)
	}
	inst, err := runner.Start(context.Background(), spec)
	if err != nil {
		t.Fatal(err)
	}
	if c, ok := inst.(interface{ CanPause() bool }); !ok || c.CanPause() {
		t.Fatal("container BDS cannot pause")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	if err := inst.Stop(ctx); err != nil {
		t.Fatal(err)
	}
	raw, _ := os.ReadFile(logPath)
	log := string(raw)
	name := "cinnabar-bds-" + spec.World.ID
	for _, want := range []string{
		"image inspect " + DefaultBDSImage,
		"pull --platform linux/amd64 " + DefaultBDSImage,
		"rm -f " + name,
		"run --rm --name " + name + " --platform linux/amd64 -p 127.0.0.1:",
		":19132/udp",
		"-v " + filepath.Join(spec.Dir, "db") + ":/data/worlds/" + spec.World.ID,
		"-e EULA=TRUE", "-e VERSION=1.26.52.3", "-e ONLINE_MODE=false", "-e LEVEL_TYPE=FLAT", "-e LEVEL_SEED=-7",
		"stop -t 25 " + name,
	} {
		if !strings.Contains(log, want) {
			t.Fatalf("docker log missing %q:\n%s", want, log)
		}
	}
	if strings.Count(log, "rm -f "+name) < 2 {
		t.Fatalf("container not removed after stop:\n%s", log)
	}
	lines := strings.Split(strings.TrimSpace(log), "\n")
	if last := lines[len(lines)-1]; last != "rm -f "+name {
		t.Fatalf("cleanup should be the last docker call, got %q", last)
	}
}

func TestContainerRunnerSkipsPullWhenImagePresent(t *testing.T) {
	env, logPath := fakeDockerEnv(t)
	_ = os.WriteFile(logPath+".pulled", nil, 0o600)
	p := macProvisioner(t)
	_ = p.AcceptEULA()
	runner := BDSRunner{Provisioner: p, Docker: os.Args[0], Env: env, StartTimeout: 20 * time.Second}
	spec := testSpec()
	spec.Dir = t.TempDir()
	inst, err := runner.Start(context.Background(), spec)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	_ = inst.Stop(ctx)
	if raw, _ := os.ReadFile(logPath); strings.Contains(string(raw), "pull ") {
		t.Fatalf("pulled an image that was present:\n%s", raw)
	}
}

func TestDetectRuntimeOrderAndUnavailableReasons(t *testing.T) {
	ctx := context.Background()
	if info := detectRuntime(ctx, "linux", "amd64", "definitely-not-docker", nil); info.Kind != RuntimeNative {
		t.Fatalf("linux = %+v", info)
	}
	if info := detectRuntime(ctx, "windows", "amd64", "definitely-not-docker", nil); info.Kind != RuntimeNative {
		t.Fatalf("windows = %+v", info)
	}
	env, _ := fakeDockerEnv(t)
	if info := detectRuntime(ctx, "darwin", "arm64", os.Args[0], env); info.Kind != RuntimeContainer || info.Unavailable != "" {
		t.Fatalf("docker up = %+v", info)
	}
	down, _ := fakeDockerEnv(t, "LOCALWORLD_TEST_DOCKER_DOWN=1")
	if info := detectRuntime(ctx, "darwin", "arm64", os.Args[0], down); info.Kind != RuntimeNone || info.Unavailable != "docker_not_running" {
		t.Fatalf("docker down = %+v", info)
	}
	if info := detectRuntime(ctx, "darwin", "arm64", "definitely-not-docker", nil); info.Kind != RuntimeNone || info.Unavailable != "docker_missing" {
		t.Fatalf("docker missing = %+v", info)
	}
	if DefaultBackend(RuntimeInfo{Kind: RuntimeContainer}) != BackendBDS || DefaultBackend(RuntimeInfo{Kind: RuntimeNone}) != BackendDragonfly {
		t.Fatal("default backend mapping wrong")
	}
}

func TestStatusExposesUnavailableReasonAndRedetectFlipsDefaultBackend(t *testing.T) {
	store := newTestStore(t)
	store.SetDefaultBackend(BackendDragonfly)
	p := &Provisioner{Root: t.TempDir(), goos: "darwin", goarch: "arm64"}
	p.SetRuntime(RuntimeInfo{Kind: RuntimeNone, Reason: "no docker", Unavailable: "docker_not_running"})
	p.SetDetector(func(context.Context) RuntimeInfo { return RuntimeInfo{Kind: RuntimeContainer, Reason: "docker up"} })
	m := NewManager(store, Runners{}, nil)
	m.SetSetup(p)
	m.SetAutoBackend(true)
	st := m.Status()
	if st.BackendUnavailableReason != "docker_not_running" || st.Setup == nil || st.Setup.State != SetupUnsupported {
		t.Fatalf("status = %+v", st)
	}
	if _, err := m.Prefs(context.Background(), PrefsUpdate{Redetect: true}); err != nil {
		t.Fatal(err)
	}
	if st := m.Status(); st.BackendUnavailableReason != "" || st.Setup.Runtime != RuntimeContainer {
		t.Fatalf("after retry = %+v", st)
	}
	world, _ := m.Create(Spec{Name: "after"})
	if world.Backend != BackendBDS {
		t.Fatalf("new world backend = %q", world.Backend)
	}
}

func TestForcedBackendSurvivesRedetectAndSavedWorldsKeepTheirs(t *testing.T) {
	store := newTestStore(t)
	store.SetDefaultBackend(BackendDragonfly)
	old, _ := store.Create(Spec{Name: "old"})
	p := &Provisioner{Root: t.TempDir(), goos: "darwin", goarch: "arm64"}
	p.SetDetector(func(context.Context) RuntimeInfo { return RuntimeInfo{Kind: RuntimeContainer} })
	m := NewManager(store, Runners{}, nil)
	m.SetSetup(p)
	_, _ = m.Prefs(context.Background(), PrefsUpdate{Redetect: true})
	if w, _ := m.Create(Spec{Name: "n"}); w.Backend != BackendDragonfly {
		t.Fatalf("forced default changed to %q", w.Backend)
	}
	m.SetAutoBackend(true)
	_, _ = m.Prefs(context.Background(), PrefsUpdate{Redetect: true})
	if got, _ := store.Get(old.ID); got.Backend != BackendDragonfly {
		t.Fatalf("saved world switched backend silently: %q", got.Backend)
	}
}

func TestPrefsPersistAndDefaultOff(t *testing.T) {
	store := newTestStore(t)
	if store.Prefs().DockerPromptDismissed {
		t.Fatal("prompt must default to shown")
	}
	yes := true
	if prefs, err := store.UpdatePrefs(PrefsUpdate{DockerPromptDismissed: &yes}); err != nil || !prefs.DockerPromptDismissed {
		t.Fatalf("update = %+v, %v", prefs, err)
	}
	reopened, _ := OpenStore(store.root)
	if !reopened.Prefs().DockerPromptDismissed {
		t.Fatal("dismissal not persisted")
	}
	if prefs, _ := reopened.UpdatePrefs(PrefsUpdate{Redetect: true}); !prefs.DockerPromptDismissed {
		t.Fatal("an update without the field must keep it")
	}
	if worlds, _ := reopened.List(); len(worlds) != 0 {
		t.Fatalf("prefs file listed as a world: %v", worlds)
	}
}
