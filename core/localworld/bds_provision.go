package localworld

import (
	"archive/zip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"time"
)

// TargetVersionPrefix is the dedicated-server line fetched for the client's pinned protocol (2193, shared by
// 1.26.50 through 1.26.52); matching is per dotted component, so "1.26.5" would not match 1.26.52.x.
const TargetVersionPrefix = "1.26.52"

const (
	linksAPI        = "https://net-secondary.web.minecraft-services.net/api/v1.0/download/links"
	directURLFormat = "https://www.minecraft.net/bedrockdedicatedserver/bin-%s/bedrock-server-%s.zip"
	maxZipBytes     = 1 << 30
	maxUnpackBytes  = 4 << 30
)

var zipVersion = regexp.MustCompile(`bedrock-server-(\d+(?:\.\d+)+)\.zip$`)

// SetupState is the lifecycle of the dedicated-server installation.
type SetupState string

const (
	SetupUnsupported  SetupState = "unsupported"
	SetupEULARequired SetupState = "eula_required"
	SetupNotInstalled SetupState = "not_installed"
	SetupDownloading  SetupState = "downloading"
	SetupUnpacking    SetupState = "unpacking"
	SetupReady        SetupState = "ready"
	SetupFailed       SetupState = "failed"
)

// SetupStatus reports BDS acquisition; Error never carries paths.
type SetupStatus struct {
	State        SetupState `json:"state"`
	Version      string     `json:"version,omitempty"`
	BytesDone    int64      `json:"bytes_done"`
	BytesTotal   int64      `json:"bytes_total"`
	EULAAccepted bool       `json:"eula_accepted"`
	Error        string     `json:"error,omitempty"`
	// Runtime is how BDS runs on this machine (native, container, none) and Reason says why.
	Runtime string `json:"runtime"`
	Reason  string `json:"reason,omitempty"`
	// UnavailableReason is docker_missing or docker_not_running when BDS cannot run only for want of Docker.
	UnavailableReason string `json:"backend_unavailable_reason,omitempty"`
}

// Setup is the dedicated-server installer as seen by the Manager.
type Setup interface {
	Status() SetupStatus
	AcceptEULA() error
	Redetect(ctx context.Context) RuntimeInfo
}

// manifest records where an installed build came from.
type manifest struct {
	Version       string `json:"version"`
	URL           string `json:"url"`
	ZipSHA256     string `json:"zip_sha256"`
	ZipBytes      int64  `json:"zip_bytes"`
	Platform      string `json:"platform"`
	DownloadedAt  int64  `json:"downloaded_unix"`
	ClientVersion string `json:"client_version_prefix"`
}

// Provisioner downloads the official Bedrock Dedicated Server on first use, after explicit EULA acceptance.
// Nothing is bundled or committed; builds live in Root/<version>/ with a provenance manifest.
type Provisioner struct {
	Root string
	// Version, when set, is an exact build (for example "1.26.52.3") fetched from its versioned official URL;
	// otherwise the download API's current build must match VersionPrefix.
	Version       string
	VersionPrefix string // default TargetVersionPrefix
	Client        *http.Client
	Log           *slog.Logger

	// test hooks
	linksURL  string
	goos      string
	goarch    string
	allowHost func(*url.URL) bool

	runtime     string // set by SetRuntime; empty derives from the platform
	reason      string
	unavailable string
	detect      func(context.Context) RuntimeInfo // used by Redetect

	ensureMu sync.Mutex
	mu       sync.Mutex
	op       SetupState // downloading or unpacking while Ensure runs
	done     int64
	total    int64
	version  string
	lastErr  string
}

func (p *Provisioner) platform() (goos, arch string) {
	goos, arch = p.goos, p.goarch
	if goos == "" {
		goos = runtime.GOOS
	}
	if arch == "" {
		arch = runtime.GOARCH
	}
	return
}

// PlatformSupportsBDS reports whether Mojang ships a dedicated server for this OS and architecture.
func PlatformSupportsBDS() bool {
	return bdsSupported(runtime.GOOS, runtime.GOARCH)
}

func bdsSupported(goos, arch string) bool {
	return (goos == "windows" || goos == "linux") && arch == "amd64"
}

// Runtimes for the dedicated server.
const (
	RuntimeNative    = "native"    // official zip run directly (Windows, Linux x86-64)
	RuntimeContainer = "container" // Linux build in a Docker-compatible container (macOS)
	RuntimeNone      = "none"
)

// SetRuntime records how BDS will run and why, as detected by DetectRuntime.
func (p *Provisioner) SetRuntime(info RuntimeInfo) {
	p.mu.Lock()
	p.runtime, p.reason, p.unavailable = info.Kind, info.Reason, info.Unavailable
	p.mu.Unlock()
}

// SetDetector sets the probe Redetect runs.
func (p *Provisioner) SetDetector(detect func(context.Context) RuntimeInfo) {
	p.mu.Lock()
	p.detect = detect
	p.mu.Unlock()
}

// Redetect re-probes the runtime (for example after the user starts Docker) and returns the result.
func (p *Provisioner) Redetect(ctx context.Context) RuntimeInfo {
	p.mu.Lock()
	detect := p.detect
	p.mu.Unlock()
	if detect == nil {
		kind, reason, unavailable := p.runtimeInfo()
		return RuntimeInfo{kind, reason, unavailable}
	}
	info := detect(ctx)
	p.SetRuntime(info)
	return info
}

func (p *Provisioner) runtimeInfo() (kind, reason, unavailable string) {
	p.mu.Lock()
	kind, reason, unavailable = p.runtime, p.reason, p.unavailable
	p.mu.Unlock()
	if kind == "" {
		goos, arch := p.platform()
		if bdsSupported(goos, arch) {
			return RuntimeNative, "native Bedrock Dedicated Server", ""
		}
		return RuntimeNone, "Bedrock Dedicated Server has no build for this platform", ""
	}
	return kind, reason, unavailable
}

func (p *Provisioner) runtimeKind() (kind, reason string) {
	kind, reason, _ = p.runtimeInfo()
	return kind, reason
}

func (p *Provisioner) prefix() string {
	if p.VersionPrefix != "" {
		return p.VersionPrefix
	}
	return TargetVersionPrefix
}

func (p *Provisioner) log() *slog.Logger {
	if p.Log != nil {
		return p.Log
	}
	return slog.Default()
}

func (p *Provisioner) binaryName() string {
	if goos, _ := p.platform(); goos == "windows" {
		return "bedrock_server.exe"
	}
	return "bedrock_server"
}

func (p *Provisioner) eulaPath() string { return filepath.Join(p.Root, "eula.json") }

func (p *Provisioner) eulaAccepted() bool {
	_, err := os.Stat(p.eulaPath())
	return err == nil
}

// AcceptEULA records the user's acceptance of the Minecraft EULA and privacy policy.
func (p *Provisioner) AcceptEULA() error {
	if err := os.MkdirAll(p.Root, 0o700); err != nil {
		return fmt.Errorf("localworld: create server directory: %w", err)
	}
	raw, _ := json.Marshal(map[string]any{"accepted_unix": time.Now().Unix(), "terms": "https://minecraft.net/eula"})
	return os.WriteFile(p.eulaPath(), raw, 0o600)
}

// installed returns the binary of an installed build whose version matches the target.
func (p *Provisioner) installed() (binary, version string, ok bool) {
	entries, err := os.ReadDir(p.Root)
	if err != nil {
		return "", "", false
	}
	for _, entry := range entries {
		if !entry.IsDir() || !p.versionMatches(entry.Name()) {
			continue
		}
		if p.Version != "" && entry.Name() != p.Version {
			continue
		}
		dir := filepath.Join(p.Root, entry.Name())
		if _, err := os.Stat(filepath.Join(dir, "manifest.json")); err != nil {
			continue
		}
		bin := filepath.Join(dir, p.binaryName())
		if info, err := os.Stat(bin); err == nil && !info.IsDir() {
			return bin, entry.Name(), true
		}
	}
	return "", "", false
}

func (p *Provisioner) versionMatches(version string) bool {
	prefix := p.prefix()
	return version == prefix || strings.HasPrefix(version, prefix+".")
}

// Status reports the installation state for the client.
func (p *Provisioner) Status() SetupStatus {
	kind, reason, unavailable := p.runtimeInfo()
	accepted := p.eulaAccepted()
	p.mu.Lock()
	op, done, total, version, lastErr := p.op, p.done, p.total, p.version, p.lastErr
	p.mu.Unlock()
	status := SetupStatus{Version: version, BytesDone: done, BytesTotal: total, EULAAccepted: accepted, Runtime: kind, Reason: reason, UnavailableReason: unavailable}
	switch {
	case kind == RuntimeNone:
		status.State = SetupUnsupported
	case op != "":
		status.State = op
	case kind == RuntimeContainer:
		switch {
		case !accepted:
			status.State = SetupEULARequired
		case lastErr != "":
			status.State, status.Error = SetupFailed, lastErr
		default:
			status.State = SetupReady
		}
	default:
		if _, v, ok := p.installed(); ok {
			status.State, status.Version = SetupReady, v
		} else if !accepted {
			status.State = SetupEULARequired
		} else if lastErr != "" {
			status.State, status.Error = SetupFailed, lastErr
		} else {
			status.State = SetupNotInstalled
		}
	}
	return status
}

func (p *Provisioner) setOp(op SetupState, version string, done, total int64) {
	p.mu.Lock()
	p.op, p.version, p.done, p.total = op, version, done, total
	p.mu.Unlock()
}

func (p *Provisioner) fail(err error) error {
	p.log().Error("dedicated server setup failed", "error", err)
	p.mu.Lock()
	p.op, p.lastErr = "", "dedicated server download failed"
	p.mu.Unlock()
	return err
}

// Ensure returns the path of the server binary, downloading and unpacking it first if needed.
func (p *Provisioner) Ensure(ctx context.Context) (string, error) {
	p.ensureMu.Lock()
	defer p.ensureMu.Unlock()
	if kind, _ := p.runtimeKind(); kind != RuntimeNative {
		return "", ErrBackendUnavailable
	}
	if !p.eulaAccepted() {
		return "", ErrEULARequired
	}
	if bin, _, ok := p.installed(); ok {
		return bin, nil
	}
	p.mu.Lock()
	p.lastErr = ""
	p.mu.Unlock()
	version, link, err := p.resolve(ctx)
	if err != nil {
		return "", p.fail(err)
	}
	zipPath, sum, size, err := p.download(ctx, version, link)
	if err != nil {
		return "", p.fail(err)
	}
	defer os.Remove(zipPath)
	p.setOp(SetupUnpacking, version, size, size)
	bin, err := p.unpack(zipPath, version, link, sum, size)
	if err != nil {
		return "", p.fail(err)
	}
	p.setOp("", version, size, size)
	return bin, nil
}

func (p *Provisioner) hostAllowed(u *url.URL) bool {
	if p.allowHost != nil {
		return p.allowHost(u)
	}
	host := strings.ToLower(u.Hostname())
	return u.Scheme == "https" && (host == "minecraft.net" || strings.HasSuffix(host, ".minecraft.net") ||
		strings.HasSuffix(host, ".minecraft-services.net"))
}

func (p *Provisioner) client() *http.Client {
	if p.Client != nil {
		return p.Client
	}
	return &http.Client{CheckRedirect: func(req *http.Request, _ []*http.Request) error {
		if !p.hostAllowed(req.URL) {
			return fmt.Errorf("localworld: refusing redirect to %s", req.URL.Host)
		}
		return nil
	}}
}

var _ Setup = (*Provisioner)(nil)

func (p *Provisioner) get(ctx context.Context, rawURL string) (*http.Response, error) {
	u, err := url.Parse(rawURL)
	if err != nil || !p.hostAllowed(u) {
		return nil, fmt.Errorf("localworld: refusing non-official download URL %q", rawURL)
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, err
	}
	resp, err := p.client().Do(req)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK {
		resp.Body.Close()
		return nil, fmt.Errorf("localworld: download %s: %s", u.Host, resp.Status)
	}
	return resp, nil
}

// resolve returns the target build's version and official zip URL.
func (p *Provisioner) resolve(ctx context.Context) (version, link string, err error) {
	goos, _ := p.platform()
	kind, dir := "serverBedrockLinux", "linux"
	if goos == "windows" {
		kind, dir = "serverBedrockWindows", "win"
	}
	if p.Version != "" {
		if !p.versionMatches(p.Version) {
			return "", "", fmt.Errorf("localworld: server version %s does not match client version %s", p.Version, p.prefix())
		}
		return p.Version, fmt.Sprintf(directURLFormat, dir, p.Version), nil
	}
	api := p.linksURL
	if api == "" {
		api = linksAPI
	}
	resp, err := p.get(ctx, api)
	if err != nil {
		return "", "", err
	}
	defer resp.Body.Close()
	var body struct {
		Result struct {
			Links []struct {
				DownloadType string `json:"downloadType"`
				DownloadURL  string `json:"downloadUrl"`
			} `json:"links"`
		} `json:"result"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<20)).Decode(&body); err != nil {
		return "", "", fmt.Errorf("localworld: decode download links: %w", err)
	}
	for _, item := range body.Result.Links {
		if item.DownloadType != kind {
			continue
		}
		match := zipVersion.FindStringSubmatch(item.DownloadURL)
		if match == nil {
			continue
		}
		if !p.versionMatches(match[1]) {
			return "", "", fmt.Errorf("localworld: current dedicated server is %s but the client needs %s; set an exact server version", match[1], p.prefix())
		}
		return match[1], item.DownloadURL, nil
	}
	return "", "", errors.New("localworld: no dedicated server download listed for this platform")
}

type progressWriter struct {
	p     *Provisioner
	ver   string
	total int64
	done  int64
	hash  io.Writer
	out   io.Writer
}

func (w *progressWriter) Write(b []byte) (int, error) {
	n, err := w.out.Write(b)
	_, _ = w.hash.Write(b[:n])
	w.done += int64(n)
	w.p.setOp(SetupDownloading, w.ver, w.done, w.total)
	if w.done > maxZipBytes {
		return n, errors.New("localworld: dedicated server download exceeds size limit")
	}
	return n, err
}

func (p *Provisioner) download(ctx context.Context, version, link string) (path, sum string, size int64, err error) {
	p.setOp(SetupDownloading, version, 0, 0)
	resp, err := p.get(ctx, link)
	if err != nil {
		return "", "", 0, err
	}
	defer resp.Body.Close()
	dir := filepath.Join(p.Root, "downloads")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", "", 0, err
	}
	file, err := os.CreateTemp(dir, "bedrock-server-*.zip.part")
	if err != nil {
		return "", "", 0, err
	}
	hash := sha256.New()
	w := &progressWriter{p: p, ver: version, total: resp.ContentLength, hash: hash, out: file}
	_, copyErr := io.Copy(w, resp.Body)
	closeErr := file.Close()
	if err := errors.Join(copyErr, closeErr); err != nil {
		os.Remove(file.Name())
		return "", "", 0, fmt.Errorf("localworld: download dedicated server: %w", err)
	}
	if resp.ContentLength > 0 && w.done != resp.ContentLength {
		os.Remove(file.Name())
		return "", "", 0, errors.New("localworld: dedicated server download was truncated")
	}
	return file.Name(), hex.EncodeToString(hash.Sum(nil)), w.done, nil
}

func (p *Provisioner) unpack(zipPath, version, link, sum string, size int64) (string, error) {
	reader, err := zip.OpenReader(zipPath)
	if err != nil {
		return "", fmt.Errorf("localworld: dedicated server archive is corrupt: %w", err)
	}
	defer reader.Close()
	final := filepath.Join(p.Root, version)
	partial := final + ".partial"
	if err := os.RemoveAll(partial); err != nil {
		return "", err
	}
	if err := os.MkdirAll(partial, 0o700); err != nil {
		return "", err
	}
	var written int64
	for _, entry := range reader.File {
		n, err := extractEntry(partial, entry, maxUnpackBytes-written)
		if err != nil {
			os.RemoveAll(partial)
			return "", err
		}
		written += n
	}
	if _, err := os.Stat(filepath.Join(partial, p.binaryName())); err != nil {
		os.RemoveAll(partial)
		return "", errors.New("localworld: dedicated server archive has no server binary")
	}
	_ = os.Chmod(filepath.Join(partial, p.binaryName()), 0o755)
	goos, arch := p.platform()
	raw, _ := json.MarshalIndent(manifest{
		Version: version, URL: link, ZipSHA256: sum, ZipBytes: size, Platform: goos + "/" + arch,
		DownloadedAt: time.Now().Unix(), ClientVersion: p.prefix(),
	}, "", "  ")
	if err := os.WriteFile(filepath.Join(partial, "manifest.json"), raw, 0o600); err != nil {
		os.RemoveAll(partial)
		return "", err
	}
	if err := os.RemoveAll(final); err != nil {
		return "", err
	}
	if err := os.Rename(partial, final); err != nil {
		return "", err
	}
	return filepath.Join(final, p.binaryName()), nil
}

// extractEntry unpacks one archive entry under root, rejecting paths that escape it; symlinks are skipped.
func extractEntry(root string, entry *zip.File, budget int64) (int64, error) {
	name := filepath.FromSlash(entry.Name)
	if filepath.IsAbs(name) || strings.HasPrefix(entry.Name, "/") {
		return 0, fmt.Errorf("localworld: archive entry %q is absolute", entry.Name)
	}
	target := filepath.Join(root, name)
	if rel, err := filepath.Rel(root, target); err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return 0, fmt.Errorf("localworld: archive entry %q escapes the install directory", entry.Name)
	}
	mode := entry.Mode()
	switch {
	case mode.IsDir():
		return 0, os.MkdirAll(target, 0o700)
	case !mode.IsRegular():
		return 0, nil
	}
	if err := os.MkdirAll(filepath.Dir(target), 0o700); err != nil {
		return 0, err
	}
	in, err := entry.Open()
	if err != nil {
		return 0, err
	}
	defer in.Close()
	out, err := os.OpenFile(target, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o600|(mode.Perm()&0o100))
	if err != nil {
		return 0, err
	}
	n, copyErr := io.Copy(out, io.LimitReader(in, budget+1))
	closeErr := out.Close()
	if err := errors.Join(copyErr, closeErr); err != nil {
		return n, err
	}
	if n > budget {
		return n, errors.New("localworld: dedicated server archive exceeds size limit")
	}
	return n, nil
}

// exactVersion returns the full BDS build the client can join (for the container runtime).
func (p *Provisioner) exactVersion(ctx context.Context) (string, error) {
	version, _, err := p.resolve(ctx)
	if err != nil {
		return "", p.fail(err)
	}
	return version, nil
}

// RuntimeInfo is the detected way to run BDS.
type RuntimeInfo struct {
	Kind, Reason string
	// Unavailable is docker_missing or docker_not_running when Kind is none for want of Docker.
	Unavailable string
}

// DetectRuntime prefers native BDS, then a Docker-compatible container (probed with `docker info`), else none.
func DetectRuntime(ctx context.Context, docker string) RuntimeInfo {
	return detectRuntime(ctx, runtime.GOOS, runtime.GOARCH, docker, nil)
}

func detectRuntime(ctx context.Context, goos, arch, docker string, env []string) RuntimeInfo {
	if bdsSupported(goos, arch) {
		return RuntimeInfo{Kind: RuntimeNative, Reason: "native Bedrock Dedicated Server"}
	}
	if docker == "" {
		docker = "docker"
	}
	if _, err := exec.LookPath(docker); err != nil {
		return RuntimeInfo{RuntimeNone, "no native Bedrock Dedicated Server for this platform and Docker is not installed; new worlds use dragonfly", "docker_missing"}
	}
	probe, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	cmd := exec.CommandContext(probe, docker, "info")
	cmd.Env = append(os.Environ(), env...)
	if err := cmd.Run(); err == nil {
		return RuntimeInfo{Kind: RuntimeContainer, Reason: "no native Bedrock Dedicated Server for this platform; running the Linux build in a container"}
	}
	return RuntimeInfo{RuntimeNone, "no native Bedrock Dedicated Server for this platform and Docker is not running; new worlds use dragonfly", "docker_not_running"}
}

// DefaultBackend is BDS when it can run (natively or in a container), else dragonfly.
func DefaultBackend(info RuntimeInfo) string {
	if info.Kind == RuntimeNative || info.Kind == RuntimeContainer {
		return BackendBDS
	}
	return BackendDragonfly
}
