package localworld

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"sync"
	"time"
)

// State is the lifecycle of the single open world.
type State string

const (
	StateIdle     State = "idle"
	StateStarting State = "starting"
	StateRunning  State = "running"
	StateStopping State = "stopping"
	StateFailed   State = "failed" // stays until Close so a failed open never falls through to another target
)

const defaultStopTimeout = 15 * time.Second

// Status is the secret-safe view of the open world; Error never carries paths.
type Status struct {
	State   State  `json:"state"`
	WorldID string `json:"world_id,omitempty"`
	Paused  bool   `json:"paused"`
	Error   string `json:"error,omitempty"`
}

// StartSpec identifies the world a Runner must host.
type StartSpec struct {
	World World
	Dir   string
}

// Instance is one running local server.
type Instance interface {
	// Address is the loopback game address.
	Address() string
	SetPaused(paused bool) error
	// Stop shuts the server down gracefully, killing it when ctx ends first.
	Stop(ctx context.Context) error
	// Done is closed once the server has exited.
	Done() <-chan struct{}
}

// Runner launches local servers; Start returns only once the server accepts connections.
type Runner interface {
	Start(ctx context.Context, spec StartSpec) (Instance, error)
}

// Manager owns the store and at most one running local world.
type Manager struct {
	store       *Store
	runner      Runner
	log         *slog.Logger
	stopTimeout time.Duration
	bg          sync.WaitGroup

	mu          sync.Mutex
	state       State
	world       World
	inst        Instance
	paused      bool
	failure     string
	cancelStart context.CancelFunc
	changed     chan struct{} // closed and replaced on every state change
}

func NewManager(store *Store, runner Runner, log *slog.Logger) *Manager {
	if log == nil {
		log = slog.Default()
	}
	return &Manager{store: store, runner: runner, log: log, stopTimeout: defaultStopTimeout, state: StateIdle, changed: make(chan struct{})}
}

func (m *Manager) setState(state State) {
	m.state = state
	close(m.changed)
	m.changed = make(chan struct{})
}

func (m *Manager) idleLocked() {
	m.world, m.inst, m.paused, m.failure, m.cancelStart = World{}, nil, false, "", nil
	m.setState(StateIdle)
}

func (m *Manager) List() ([]World, error) { return m.store.List() }

func (m *Manager) Create(spec Spec) (World, error) { return m.store.Create(spec) }

func (m *Manager) Rename(id, name string) (World, error) { return m.store.Rename(id, name) }

// Delete removes a world that is not starting, running or stopping.
func (m *Manager) Delete(id string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.state != StateIdle && m.state != StateFailed && m.world.ID == id {
		return ErrInUse
	}
	return m.store.Delete(id)
}

// Status reports the open world's lifecycle.
func (m *Manager) Status() Status {
	m.mu.Lock()
	defer m.mu.Unlock()
	return Status{State: m.state, WorldID: m.world.ID, Paused: m.paused, Error: m.failure}
}

// Open begins starting a world and returns immediately; poll Status for readiness.
// Reopening the world that is already starting or running is a no-op.
func (m *Manager) Open(id string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	switch m.state {
	case StateStarting, StateRunning:
		if m.world.ID == id {
			return nil
		}
		return ErrBusy
	case StateStopping:
		return ErrBusy
	}
	world, err := m.store.Get(id)
	if err != nil {
		return err
	}
	dir, err := m.store.Dir(id)
	if err != nil {
		return err
	}
	ctx, cancel := context.WithCancel(context.Background())
	m.world, m.failure, m.paused, m.cancelStart = world, "", false, cancel
	m.setState(StateStarting)
	m.bg.Add(1)
	go m.start(ctx, StartSpec{World: world, Dir: dir})
	return nil
}

func (m *Manager) start(ctx context.Context, spec StartSpec) {
	defer m.bg.Done()
	inst, err := m.runner.Start(ctx, spec)
	m.mu.Lock()
	if ctx.Err() != nil {
		m.mu.Unlock()
		if inst != nil {
			m.stopInstance(inst)
		}
		m.mu.Lock()
		m.idleLocked()
		m.mu.Unlock()
		return
	}
	if err != nil {
		m.log.Error("local world server failed to start", "world", spec.World.ID, "error", err)
		m.failure = "local world server failed to start"
		m.setState(StateFailed)
		m.mu.Unlock()
		return
	}
	m.inst = inst
	paused := m.paused
	m.setState(StateRunning)
	m.mu.Unlock()
	if err := m.store.Touch(spec.World.ID); err != nil {
		m.log.Warn("record last played failed", "world", spec.World.ID, "error", err)
	}
	if paused {
		_ = inst.SetPaused(true)
	}
	m.bg.Add(1)
	go m.watch(inst)
}

func (m *Manager) watch(inst Instance) {
	defer m.bg.Done()
	<-inst.Done()
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.inst == inst && m.state == StateRunning {
		m.log.Error("local world server exited unexpectedly", "world", m.world.ID)
		m.inst = nil
		m.failure = "local world server exited unexpectedly"
		m.setState(StateFailed)
	}
}

func (m *Manager) stopInstance(inst Instance) {
	ctx, cancel := context.WithTimeout(context.Background(), m.stopTimeout)
	defer cancel()
	if err := inst.Stop(ctx); err != nil {
		m.log.Warn("stop local world server", "error", err)
	}
}

// Close stops the open world (saving it) or clears a failure; it returns without waiting for shutdown.
func (m *Manager) Close() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	switch m.state {
	case StateFailed:
		m.idleLocked()
	case StateStarting:
		m.cancelStart()
		m.setState(StateStopping)
	case StateRunning:
		inst := m.inst
		m.inst = nil
		m.setState(StateStopping)
		m.bg.Add(1)
		go func() {
			defer m.bg.Done()
			m.stopInstance(inst)
			m.mu.Lock()
			m.idleLocked()
			m.mu.Unlock()
		}()
	}
	return nil
}

// Shutdown closes the open world and waits until its server has exited.
func (m *Manager) Shutdown() {
	_ = m.Close()
	m.bg.Wait()
}

// SetPaused freezes or resumes the running world; a request while starting applies once it is running.
func (m *Manager) SetPaused(paused bool) error {
	m.mu.Lock()
	switch m.state {
	case StateStarting:
		m.paused = paused
		m.mu.Unlock()
		return nil
	case StateRunning:
		m.paused = paused
		inst := m.inst
		m.mu.Unlock()
		return inst.SetPaused(paused)
	}
	m.mu.Unlock()
	return ErrNotOpen
}

// Target reports the local server address for the proxy, waiting out a start. ok is false when no world is open.
func (m *Manager) Target(ctx context.Context) (address string, ok bool, err error) {
	for {
		m.mu.Lock()
		state, inst, changed, failure := m.state, m.inst, m.changed, m.failure
		m.mu.Unlock()
		switch state {
		case StateIdle:
			return "", false, nil
		case StateRunning:
			return inst.Address(), true, nil
		case StateFailed:
			return "", false, fmt.Errorf("local world unavailable: %s", failure)
		case StateStopping:
			return "", false, errors.New("local world is closing")
		}
		select {
		case <-changed:
		case <-ctx.Done():
			return "", false, ctx.Err()
		}
	}
}
