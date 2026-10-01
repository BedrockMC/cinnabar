package proxy

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"

	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

const (
	formSchemaByteLimit  = 64 * 1024
	formSchemaNodeLimit  = 128
	formSchemaDepthLimit = 16
	formSchemaKeyLimit   = 32
)

// The endpoint-valued opt-in deliberately leaves the app's boolean probe off.
// This process owns one attempt, including refusals, across all dialers.
var formSchemaProcess struct {
	sync.Once
	probe *formSchemaProbe
}

type formSchemaProbe struct {
	endpoint *net.UDPAddr
	claimed  atomic.Bool
	emit     func(string)
}

func processFormSchemaProbe() *formSchemaProbe {
	formSchemaProcess.Do(func() {
		endpoint := formSchemaEndpoint(os.Getenv("RUST_MCBE_FORM_SHAPE_PROBE"))
		if endpoint == nil {
			return
		}
		output := formSchemaOutput(os.Getenv("RUST_MCBE_FORM_SCHEMA_OUTPUT"))
		if endpoint != nil && output != "" {
			formSchemaProcess.probe = &formSchemaProbe{endpoint: endpoint, emit: func(record string) {
				writeFormSchemaFile(output, record)
			}}
		}
	})
	return formSchemaProcess.probe
}

// Only an operator-created, real local parent and an absent regular-file
// destination are eligible. Exclusive creation remains the final race guard.
func formSchemaOutput(value string) string {
	if value == "" || len(value) > 1024 || !filepath.IsAbs(value) || filepath.Clean(value) != value || strings.HasPrefix(value, `\\`) {
		return ""
	}
	for _, r := range value {
		if r < 32 || r == 127 {
			return ""
		}
	}
	volume := filepath.VolumeName(value)
	if strings.Contains(value[len(volume):], ":") {
		return ""
	}
	for _, part := range strings.FieldsFunc(value[len(volume):], func(r rune) bool { return r == '/' || r == '\\' }) {
		if strings.HasSuffix(part, ".") || strings.HasSuffix(part, " ") {
			return ""
		}
		base := strings.ToUpper(strings.SplitN(part, ".", 2)[0])
		if base == "CON" || base == "PRN" || base == "AUX" || base == "NUL" || base == "CONIN$" || base == "CONOUT$" || len(base) == 4 && (strings.HasPrefix(base, "COM") || strings.HasPrefix(base, "LPT")) && base[3] >= '0' && base[3] <= '9' {
			return ""
		}
	}
	for parent := filepath.Dir(value); ; parent = filepath.Dir(parent) {
		info, err := os.Lstat(parent)
		if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return ""
		}
		if parent == filepath.Dir(parent) {
			break
		}
	}
	if _, err := os.Lstat(value); !os.IsNotExist(err) {
		return ""
	}
	return value
}

func writeFormSchemaFile(path, record string) {
	if len(record) > 16*1024 || formSchemaOutput(path) == "" {
		return
	}
	parent := filepath.Dir(path)
	before, err := os.Lstat(parent)
	if err != nil {
		return
	}
	root, err := os.OpenRoot(parent)
	if err != nil {
		return
	}
	defer root.Close()
	after, err := root.Stat(".")
	if err != nil || !os.SameFile(before, after) {
		return
	}
	file, err := root.OpenFile(filepath.Base(path), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return
	}
	writeFormSchemaRecord(file, record)
}

func writeFormSchemaRecord(file io.WriteCloser, record string) {
	defer file.Close()
	if len(record) > 16*1024 {
		return
	}
	_, _ = io.WriteString(file, record)
}

func formSchemaEndpoint(value string) *net.UDPAddr {
	if len(value) > 128 {
		return nil
	}
	host, portText, err := net.SplitHostPort(value)
	if err != nil {
		return nil
	}
	ip := net.ParseIP(host)
	port, err := strconv.ParseUint(portText, 10, 16)
	if err != nil || port == 0 || ip == nil || !ip.IsLoopback() {
		return nil
	}
	return &net.UDPAddr{IP: ip, Port: int(port)}
}

func (probe *formSchemaProbe) observe(header packet.Header, payload []byte, source, destination net.Addr) {
	if probe == nil {
		return
	}
	defer func() {
		if recover() != nil {
			probe.claimed.Store(true)
		}
	}()
	if probe.claimed.Load() {
		return
	}
	src, ok := source.(*net.UDPAddr)
	dst, dstOK := destination.(*net.UDPAddr)
	if !ok || !dstOK || src.Zone != "" || dst.Zone != "" || !src.IP.Equal(probe.endpoint.IP) || src.Port != probe.endpoint.Port || !dst.IP.IsLoopback() || (dst.IP.Equal(src.IP) && dst.Port == src.Port) {
		return
	}
	if header.PacketID == packet.IDTransfer {
		probe.claimed.Store(true)
		return
	}
	if header.PacketID != packet.IDModalFormRequest || !probe.claimed.CompareAndSwap(false, true) {
		return
	}
	// Diagnostics cannot interrupt the normal callback or alter its payload.
	record := formSchemaRecord(payload)
	probe.emit(record)
}

func formSchemaRecord(payload []byte) string {
	if len(payload) > formSchemaByteLimit+10 {
		return "refused=bytes"
	}
	var request packet.ModalFormRequest
	if !decodeObserved(&request, payload) {
		return "refused=wire"
	}
	if len(request.FormData) > formSchemaByteLimit {
		return "refused=bytes"
	}
	decoder := json.NewDecoder(bytes.NewReader(request.FormData))
	decoder.UseNumber()
	walk := formSchemaWalk{decoder: decoder}
	if !walk.value("", -1, 0) {
		return "refused=shape"
	}
	if _, err := decoder.Token(); err != io.EOF {
		return "refused=shape"
	}
	return walk.record.String()
}

type formSchemaWalk struct {
	decoder *json.Decoder
	nodes   int
	record  strings.Builder
}

func safeFormSchemaKey(value string) bool {
	if len(value) == 0 || len(value) > formSchemaKeyLimit {
		return false
	}
	for _, b := range []byte(value) {
		if !(b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' || b == '_' || b == '-') {
			return false
		}
	}
	return true
}

func formSchemaClass(value string) string {
	switch value {
	case "form", "modal", "custom_form", "button", "header", "label", "divider", "path", "url":
		return value
	default:
		return "Other"
	}
}

func (walk *formSchemaWalk) value(key string, parent, depth int) bool {
	if depth > formSchemaDepthLimit || walk.nodes >= formSchemaNodeLimit {
		return false
	}
	token, err := walk.decoder.Token()
	if err != nil {
		return false
	}
	index := walk.nodes
	walk.nodes++
	fmt.Fprintf(&walk.record, "node=%d parent=%d key=%s ", index, parent, key)
	switch token := token.(type) {
	case json.Delim:
		if token != '{' && token != '[' {
			return false
		}
		kind, closing := "object", json.Delim('}')
		if token == '[' {
			kind, closing = "array", ']'
		}
		fmt.Fprintf(&walk.record, "kind=%s;", kind)
		count := 0
		for walk.decoder.More() {
			childKey := ""
			if token == '{' {
				field, err := walk.decoder.Token()
				var ok bool
				childKey, ok = field.(string)
				if err != nil || !ok || !safeFormSchemaKey(childKey) {
					return false
				}
			}
			if !walk.value(childKey, index, depth+1) {
				return false
			}
			count++
		}
		end, err := walk.decoder.Token()
		if err != nil || end != closing {
			return false
		}
		fmt.Fprintf(&walk.record, "container=%d count=%d;", index, count)
	case string:
		fmt.Fprintf(&walk.record, "kind=string bytes=%d", len(token))
		if key == "type" {
			fmt.Fprintf(&walk.record, " class=%s", formSchemaClass(token))
		}
		walk.record.WriteByte(';')
	case json.Number:
		walk.record.WriteString("kind=number;")
	case bool:
		walk.record.WriteString("kind=boolean;")
	case nil:
		walk.record.WriteString("kind=null;")
	default:
		return false
	}
	return walk.record.Len() <= 16*1024
}
