package catalog

import (
	"net/http"

	"github.com/hashimthearab/rust-mcbe/core/internal/locale"
)

// messagingHTTPClient adds the active UI language to messaging requests.
func messagingHTTPClient(client *http.Client, language string) *http.Client {
	if client == nil {
		client = http.DefaultClient
	}
	copy := *client
	transport := client.Transport
	if transport == nil {
		transport = http.DefaultTransport
	}
	if language == "" {
		language = locale.Default
	}
	copy.Transport = messagingLanguage{transport: transport, language: language}
	return &copy
}

type messagingLanguage struct {
	transport http.RoundTripper
	language  string
}

// RoundTrip sets the locale without mutating the caller's request.
func (m messagingLanguage) RoundTrip(request *http.Request) (*http.Response, error) {
	copy := request.Clone(request.Context())
	copy.Header.Set("Accept-Language", m.language)
	return m.transport.RoundTrip(copy)
}
