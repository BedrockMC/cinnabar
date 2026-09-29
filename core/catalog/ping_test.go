package catalog

import (
	"context"
	"errors"
	"testing"
)

func TestPongsParseLenientlyAndFailuresReadOffline(t *testing.T) {
	ping := func(_ context.Context, address string) ([]byte, error) {
		switch address {
		case "a.test:19132":
			return []byte("MCPE;A Server;900;1.26.30;12;100;123;sub;Survival;1;19132;19133;"), nil
		case "b.test:19133":
			return []byte("MCPE;short"), nil
		}
		return nil, errors.New("timeout")
	}
	results := pingWith(context.Background(), []string{"a.test", "b.test:19133", "c.test:1"}, ping)
	if !results[0].Online || results[0].Players != 12 || results[0].MaxPlayers != 100 || results[0].MOTD != "A Server" {
		t.Fatalf("a = %+v", results[0])
	}
	if results[1].Online || results[2].Online || results[2].Address != "c.test:1" {
		t.Fatalf("b/c = %+v %+v", results[1], results[2])
	}
}

func TestDefaultPortsBracketIPv6(t *testing.T) {
	if got := withDefaultPort("::1"); got != "[::1]:19132" {
		t.Fatalf("got %q", got)
	}
	if got := withDefaultPort("[::1]:5"); got != "[::1]:5" {
		t.Fatalf("got %q", got)
	}
}
