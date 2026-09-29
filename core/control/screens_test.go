package control

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/hashimthearab/rust-mcbe/core/catalog"
)

type stubScreens struct {
	stubServices
}

func (stubScreens) FeaturedServers(context.Context) ([]catalog.FeaturedServer, error) {
	return []catalog.FeaturedServer{{Name: "Example", Address: "play.example.test:19132"}}, nil
}

func (stubScreens) Gatherings(context.Context) ([]catalog.Gathering, error) { return nil, nil }

func (stubScreens) Profile(context.Context) (catalog.Profile, error) {
	return catalog.Profile{Gamertag: "Steve", XUID: "1"}, nil
}

func (stubScreens) Ping(_ context.Context, addresses []string) []catalog.PingResult {
	results := make([]catalog.PingResult, len(addresses))
	for index, address := range addresses {
		results[index] = catalog.PingResult{Address: address, Online: true, Players: 3}
	}
	return results
}

func TestScreenFeedsServeWhenTheBackendSupportsThem(t *testing.T) {
	dir := startServices(t, NewStore(), &stubScreens{})
	var featured featuredServersResultV1
	if reply := rpc(t, dir, methodFeaturedServers, ""); reply.Error != nil || json.Unmarshal(reply.Result, &featured) != nil ||
		len(featured.Servers) != 1 || featured.Servers[0].Address != "play.example.test:19132" {
		t.Fatalf("featured = %+v / %+v", featured, reply.Error)
	}
	var gatherings gatheringsResultV1
	if reply := rpc(t, dir, methodGatherings, ""); reply.Error != nil || json.Unmarshal(reply.Result, &gatherings) != nil ||
		gatherings.Gatherings == nil {
		t.Fatalf("empty gatherings must encode as an array: %s", reply.Result)
	}
	var profile profileResultV1
	if reply := rpc(t, dir, methodProfile, ""); reply.Error != nil || json.Unmarshal(reply.Result, &profile) != nil ||
		profile.Profile.Gamertag != "Steve" {
		t.Fatalf("profile = %+v / %+v", profile, reply.Error)
	}
	if reply := rpc(t, dir, methodProfile, `{"x":1}`); reply.Error == nil || reply.Error.Code != -32602 {
		t.Fatalf("params must be rejected: %+v", reply.Error)
	}
	var pinged pingResultV1
	if reply := rpc(t, dir, methodPing, `{"addresses":["a.test:19132"]}`); reply.Error != nil ||
		json.Unmarshal(reply.Result, &pinged) != nil || len(pinged.Servers) != 1 || pinged.Servers[0].Players != 3 {
		t.Fatalf("ping = %+v / %+v", pinged, reply.Error)
	}
	if reply := rpc(t, dir, methodPing, ""); reply.Error == nil || reply.Error.Code != -32602 {
		t.Fatalf("ping needs addresses: %+v", reply.Error)
	}
}

func TestScreenFeedsNeedAScreenBackend(t *testing.T) {
	dir := startServices(t, NewStore(), &stubServices{})
	if reply := rpc(t, dir, methodGatherings, ""); reply.Error == nil || reply.Error.Code != codeServicesDisabled {
		t.Fatalf("reply = %+v", reply.Error)
	}
}
