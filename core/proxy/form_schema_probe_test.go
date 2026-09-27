package proxy

import (
	"bytes"
	"errors"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/sandertv/gophertunnel/minecraft/protocol/login"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

func schemaPayload(text string) []byte {
	payload := []byte{1}
	length := uint32(len(text))
	for length >= 128 {
		payload = append(payload, byte(length)|128)
		length >>= 7
	}
	payload = append(payload, byte(length))
	return append(payload, text...)
}

func TestFormSchemaEndpointFailsClosed(t *testing.T) {
	for _, value := range []string{"", "1", "localhost:19132", "example.test:19132", "192.0.2.1:19132", "127.0.0.1:0", "[::1%zone]:19132", strings.Repeat("a", 129)} {
		if formSchemaEndpoint(value) != nil {
			t.Fatal("unsafe endpoint accepted")
		}
	}
	if formSchemaEndpoint("127.0.0.1:19132") == nil || formSchemaEndpoint("[::1]:19132") == nil {
		t.Fatal("numeric loopback rejected")
	}
}

func TestFormSchemaProbeDirectionTransferAndUnchangedPayload(t *testing.T) {
	server := formSchemaEndpoint("127.0.0.1:19132")
	local := formSchemaEndpoint("127.0.0.1:20000")
	other := &net.UDPAddr{IP: net.ParseIP("192.0.2.1"), Port: 19132}
	payload := schemaPayload(`{"type":"form","title":"secret","content":"https://private.invalid","replacement":[{"type":"button","text":"private-one"},{"type":"button","text":"private-two"}]}`)
	original := bytes.Clone(payload)
	emissions := 0
	probe := &formSchemaProbe{endpoint: server, emit: func(string) { emissions++ }}
	var disabled *formSchemaProbe
	disabled.observe(packet.Header{PacketID: packet.IDModalFormRequest}, payload, server, local)
	probe.observe(packet.Header{PacketID: packet.IDTransfer}, nil, local, server)
	probe.observe(packet.Header{PacketID: packet.IDModalFormRequest}, payload, local, server)
	probe.observe(packet.Header{PacketID: packet.IDModalFormRequest}, payload, other, local)
	if probe.claimed.Load() || emissions != 0 {
		t.Fatal("unmatched or outbound packet claimed probe")
	}
	probe.observe(packet.Header{PacketID: packet.IDModalFormRequest}, payload, server, local)
	probe.observe(packet.Header{PacketID: packet.IDModalFormRequest}, payload, server, local)
	if emissions != 1 || !bytes.Equal(original, payload) {
		t.Fatal("one-shot observation changed forwarding bytes")
	}
	transferred := &formSchemaProbe{endpoint: server, emit: func(string) { t.Fatal("transfer must permanently disarm") }}
	transferred.observe(packet.Header{PacketID: packet.IDTransfer}, nil, server, local)
	transferred.observe(packet.Header{PacketID: packet.IDModalFormRequest}, payload, server, local)
}

func TestFormSchemaNestedKeysDiscriminatorsAndPrivacy(t *testing.T) {
	record := formSchemaRecord(schemaPayload(`{"type":"form","title":"hidden-title","replacement":[{"type":"button","text":"hidden-text","image":{"type":"url","data":"https://hidden.invalid/path"}},{"type":"header","label":"hidden-label"},{"type":"arbitrary-secret","flag":true,"value":42,"nullable":null}]}`))
	for _, expected := range []string{"key=replacement kind=array", "class=form", "class=button", "class=header", "class=url", "class=Other", "key=data kind=string bytes=", "count=3", "kind=boolean", "kind=number", "kind=null"} {
		if !strings.Contains(record, expected) {
			t.Fatalf("missing fixed structural observation %q", expected)
		}
	}
	for _, secret := range []string{"hidden-title", "hidden-text", "hidden-label", "hidden.invalid", "arbitrary-secret", "https://", "42"} {
		if strings.Contains(record, secret) {
			t.Fatal("record exposed value")
		}
	}
}

func TestFormSchemaHostileInputsAndAttemptBudget(t *testing.T) {
	for _, text := range []string{
		`{"unsafe/key":0}`, `{"é":0}`, `{"":0}`, `{"` + strings.Repeat("k", 33) + `":0}`,
		strings.Repeat("[", 18) + "0" + strings.Repeat("]", 18),
		"[" + strings.Repeat("0,", 128) + "0]", `{"a":`, `{} {}`, `{"a":1e999999}` + "x",
	} {
		if formSchemaRecord(schemaPayload(text)) != "refused=shape" {
			t.Fatal("hostile structure was not refused")
		}
	}
	for _, payload := range [][]byte{{128}, {1, 255}, {1, 0, 1}, schemaPayload(strings.Repeat(" ", formSchemaByteLimit+1))} {
		if !strings.HasPrefix(formSchemaRecord(payload), "refused=") {
			t.Fatal("bad wire or byte limit accepted")
		}
	}
	server, local := formSchemaEndpoint("127.0.0.1:19132"), formSchemaEndpoint("127.0.0.1:20000")
	count := 0
	probe := &formSchemaProbe{endpoint: server, emit: func(record string) {
		count++
		if record != "refused=wire" {
			t.Fatal("expected refusal")
		}
	}}
	probe.observe(packet.Header{PacketID: packet.IDModalFormRequest}, []byte{128}, server, local)
	probe.observe(packet.Header{PacketID: packet.IDModalFormRequest}, schemaPayload(`{}`), server, local)
	if count != 1 {
		t.Fatal("refusal reminted attempt")
	}
}

func TestFormSchemaSharedProcessBudgetAndObserverPanicIsolation(t *testing.T) {
	server, local := formSchemaEndpoint("127.0.0.1:19132"), formSchemaEndpoint("127.0.0.1:20000")
	var count int
	probe := &formSchemaProbe{endpoint: server, emit: func(string) { count++; panic("not logged") }}
	// All dialers retain the same pointer, rather than copying atomic ownership.
	aliases := []*formSchemaProbe{probe, probe, probe}
	var workers sync.WaitGroup
	for _, alias := range aliases {
		workers.Go(func() {
			alias.observe(packet.Header{PacketID: packet.IDModalFormRequest}, schemaPayload(`{}`), server, local)
		})
	}
	workers.Wait()
	if count != 1 || !probe.claimed.Load() {
		t.Fatal("shared attempt not owned once")
	}
}

type failedSchemaWriter struct{ closed bool }

func (*failedSchemaWriter) Write([]byte) (int, error) { return 0, errors.New("fixed failure") }
func (writer *failedSchemaWriter) Close() error {
	writer.closed = true
	return errors.New("fixed failure")
}

func TestFormSchemaExclusiveLocalFileAndWriteFailure(t *testing.T) {
	parent := t.TempDir()
	output := filepath.Join(parent, "record.txt")
	for _, invalid := range []string{"", "relative.txt", `\\server\share\record.txt`, `\\?\C:\record.txt`, filepath.Join(parent, "NUL"), filepath.Join(parent, "missing", "record.txt"), filepath.Join(parent, "record.txt:") + "stream"} {
		if formSchemaOutput(invalid) != "" {
			t.Fatal("unsafe file configuration accepted")
		}
	}
	if formSchemaOutput(output) != output {
		t.Fatal("precreated local parent rejected")
	}
	writeFormSchemaFile(output, "kind=object;")
	writeFormSchemaFile(output, "overwrite")
	actual, err := os.ReadFile(output)
	if err != nil || string(actual) != "kind=object;" {
		t.Fatal("exclusive output changed existing file")
	}
	if formSchemaOutput(output) != "" {
		t.Fatal("existing output eligible")
	}
	failed := &failedSchemaWriter{}
	writeFormSchemaRecord(failed, "kind=object;")
	if !failed.closed {
		t.Fatal("write failure did not close")
	}
	oversized := filepath.Join(parent, "oversized.txt")
	writeFormSchemaFile(oversized, strings.Repeat("x", 16*1024+1))
	if _, err := os.Stat(oversized); !os.IsNotExist(err) {
		t.Fatal("oversized output created")
	}
	server, local := formSchemaEndpoint("127.0.0.1:19132"), formSchemaEndpoint("127.0.0.1:20000")
	blocked := &formSchemaProbe{endpoint: server, emit: func(record string) { writeFormSchemaFile(output, record) }}
	blocked.observe(packet.Header{PacketID: packet.IDModalFormRequest}, schemaPayload(`{}`), server, local)
	if !blocked.claimed.Load() {
		t.Fatal("failed capture reminted attempt")
	}
	actual, _ = os.ReadFile(output)
	if string(actual) != "kind=object;" {
		t.Fatal("failed capture overwrote file")
	}
}

func TestFormSchemaFreshProcessConfiguration(t *testing.T) {
	for _, mode := range []string{"disabled", "missing-output", "enabled"} {
		output := filepath.Join(t.TempDir(), "schema.txt")
		command := exec.Command(os.Args[0], "-test.run=^TestFormSchemaProcessChild$")
		for _, entry := range os.Environ() {
			if !strings.HasPrefix(entry, "RUST_MCBE_FORM_SHAPE_PROBE=") && !strings.HasPrefix(entry, "RUST_MCBE_FORM_SCHEMA_OUTPUT=") && !strings.HasPrefix(entry, "FORM_SCHEMA_TEST_CHILD=") {
				command.Env = append(command.Env, entry)
			}
		}
		endpoint := ""
		if mode != "disabled" {
			endpoint = "127.0.0.1:19132"
		}
		configuredOutput := output
		if mode == "missing-output" {
			configuredOutput = ""
		}
		command.Env = append(command.Env, "FORM_SCHEMA_TEST_CHILD="+mode, "RUST_MCBE_FORM_SHAPE_PROBE="+endpoint, "RUST_MCBE_FORM_SCHEMA_OUTPUT="+configuredOutput)
		if _, err := command.CombinedOutput(); err != nil {
			t.Fatal("isolated configuration witness failed")
		}
		data, err := os.ReadFile(output)
		if mode == "enabled" {
			if err != nil || !strings.Contains(string(data), "key=replacement kind=array") {
				t.Fatal("production callback failed to capture schema")
			}
		} else if !os.IsNotExist(err) {
			t.Fatal("disabled configuration wrote output")
		}
	}
}

func TestFormSchemaProcessChild(t *testing.T) {
	mode := os.Getenv("FORM_SCHEMA_TEST_CHILD")
	if mode == "" {
		return
	}
	first := processFormSchemaProbe()
	if mode != "enabled" {
		if first != nil {
			t.Fatal("disabled configuration created observer")
		}
		return
	}
	if first == nil || processFormSchemaProbe() != first {
		t.Fatal("process ownership changed across dialers")
	}
	downstream := dialerTestDownstream{client: login.ClientData{ServerAddress: "controlled.test:19132"}}
	one := newUpstreamDialerForAdmission(downstream, nil, nil, nil, nil, false)
	two := newUpstreamDialerForAdmission(downstream, nil, nil, nil, nil, false)
	if !reflect.DeepEqual(one.ClientData, downstream.client) || one.Protocol != downstream.protocol {
		t.Fatal("observer changed downstream client settings")
	}
	server, local := formSchemaEndpoint("127.0.0.1:19132"), formSchemaEndpoint("127.0.0.1:20000")
	payload := schemaPayload(`{"replacement":[{"type":"button","text":"hidden"}]}`)
	before := bytes.Clone(payload)
	one.PacketFunc(packet.Header{PacketID: packet.IDModalFormRequest}, payload, server, local)
	two.PacketFunc(packet.Header{PacketID: packet.IDModalFormRequest}, schemaPayload(`{"second":true}`), server, local)
	if !bytes.Equal(payload, before) || !first.claimed.Load() {
		t.Fatal("production callback changed payload or reminted budget")
	}
}
