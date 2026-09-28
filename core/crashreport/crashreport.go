// Package crashreport uploads opt-in crash reports to a Sentry-compatible ingest endpoint.
package crashreport

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"
)

const maxReportBytes = 512 << 10

// Report is the on-disk crash record written by the client on panic.
type Report struct {
	Source    string `json:"source"` // "client" or "core"
	Message   string `json:"message"`
	Backtrace string `json:"backtrace,omitempty"`
	LogTail   string `json:"log_tail,omitempty"`
	Release   string `json:"release"`
	OS        string `json:"os"`
	Arch      string `json:"arch"`
}

// DSN is a parsed Sentry DSN.
type DSN struct {
	Key     string
	Ingest  string
	Project string
}

// ParseDSN accepts https://<key>@<host>/<project> and rejects anything else.
func ParseDSN(raw string) (DSN, error) {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "https" || u.User == nil || u.User.Username() == "" || u.Host == "" {
		return DSN{}, errors.New("invalid crash-report DSN")
	}
	project := strings.Trim(u.Path, "/")
	if project == "" || strings.Contains(project, "/") {
		return DSN{}, errors.New("invalid crash-report DSN project")
	}
	return DSN{Key: u.User.Username(), Ingest: u.Scheme + "://" + u.Host, Project: project}, nil
}

// Upload reads the report file and posts it. The file is not deleted; callers do that on success.
func Upload(ctx context.Context, dsn DSN, path, home string, client *http.Client) error {
	data, err := readCapped(path)
	if err != nil {
		return err
	}
	var report Report
	if err := json.Unmarshal(data, &report); err != nil {
		return fmt.Errorf("decode crash report: %w", err)
	}
	body, err := Envelope(report, home, time.Now())
	if err != nil {
		return err
	}
	if client == nil {
		client = &http.Client{Timeout: 20 * time.Second}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, dsn.Ingest+"/api/"+dsn.Project+"/envelope/", bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/x-sentry-envelope")
	req.Header.Set("X-Sentry-Auth", "Sentry sentry_version=7, sentry_client=cinnabar/1, sentry_key="+dsn.Key)
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("upload crash report: %w", err)
	}
	defer resp.Body.Close()
	_, _ = io.Copy(io.Discard, io.LimitReader(resp.Body, 4096))
	if resp.StatusCode/100 != 2 {
		return fmt.Errorf("upload crash report: status %d", resp.StatusCode)
	}
	return nil
}

// Envelope builds the Sentry envelope bytes with the user's home directory scrubbed from all text.
func Envelope(r Report, home string, now time.Time) ([]byte, error) {
	id := make([]byte, 16)
	if _, err := rand.Read(id); err != nil {
		return nil, err
	}
	eventID := hex.EncodeToString(id)
	scrub := func(s string) string {
		if home != "" && len(home) > 1 {
			s = strings.ReplaceAll(s, home, "~")
		}
		return s
	}
	event := map[string]any{
		"event_id":  eventID,
		"timestamp": now.UTC().Format(time.RFC3339),
		"platform":  "native",
		"level":     "fatal",
		"release":   r.Release,
		"tags":      map[string]string{"source": r.Source, "os": r.OS, "arch": r.Arch},
		"exception": map[string]any{"values": []map[string]string{{"type": "panic", "value": scrub(r.Message)}}},
		"extra":     map[string]string{"backtrace": scrub(r.Backtrace), "log_tail": scrub(r.LogTail)},
	}
	payload, err := json.Marshal(event)
	if err != nil {
		return nil, err
	}
	header, _ := json.Marshal(map[string]string{"event_id": eventID})
	item, _ := json.Marshal(map[string]any{"type": "event", "length": len(payload)})
	var out bytes.Buffer
	for _, part := range [][]byte{header, item, payload} {
		out.Write(part)
		out.WriteByte('\n')
	}
	return out.Bytes(), nil
}

func readCapped(path string) ([]byte, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	data, err := io.ReadAll(io.LimitReader(f, maxReportBytes+1))
	if err != nil {
		return nil, err
	}
	if len(data) > maxReportBytes {
		return nil, errors.New("crash report exceeds size limit")
	}
	return data, nil
}
