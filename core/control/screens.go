package control

import (
	"context"

	"github.com/hashimthearab/rust-mcbe/core/catalog"
)

// Menu screen feeds served beside the account methods.
const (
	methodFeaturedServers = "featured_servers.v1"
	methodGatherings      = "gatherings.v1"
	methodProfile         = "profile.v1"
)

// ScreenServices feeds the start and play screens; a Services value may implement it.
type ScreenServices interface {
	FeaturedServers(ctx context.Context) ([]catalog.FeaturedServer, error)
	Gatherings(ctx context.Context) ([]catalog.Gathering, error)
	Profile(ctx context.Context) (catalog.Profile, error)
}

type featuredServersResultV1 struct {
	SchemaVersion uint32                   `json:"schema_version"`
	Servers       []catalog.FeaturedServer `json:"servers"`
}

type gatheringsResultV1 struct {
	SchemaVersion uint32              `json:"schema_version"`
	Gatherings    []catalog.Gathering `json:"gatherings"`
}

type profileResultV1 struct {
	SchemaVersion uint32          `json:"schema_version"`
	Profile       catalog.Profile `json:"profile"`
}

func isScreenMethod(method string) bool {
	switch method {
	case methodFeaturedServers, methodGatherings, methodProfile:
		return true
	}
	return false
}

func screenResult(ctx context.Context, screens ScreenServices, method string) (any, error) {
	switch method {
	case methodFeaturedServers:
		servers, err := screens.FeaturedServers(ctx)
		if servers == nil {
			servers = []catalog.FeaturedServer{}
		}
		return featuredServersResultV1{SchemaVersion: 1, Servers: servers}, err
	case methodGatherings:
		gatherings, err := screens.Gatherings(ctx)
		if gatherings == nil {
			gatherings = []catalog.Gathering{}
		}
		return gatheringsResultV1{SchemaVersion: 1, Gatherings: gatherings}, err
	default:
		profile, err := screens.Profile(ctx)
		return profileResultV1{SchemaVersion: 1, Profile: profile}, err
	}
}
