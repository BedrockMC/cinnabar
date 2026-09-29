package catalog

import (
	"context"
	"errors"

	"golang.org/x/oauth2"
)

// Realms lists the account's Realms with their join targets.
func Realms(ctx context.Context, src oauth2.TokenSource) ([]Realm, error) {
	if src == nil {
		return nil, errors.New("catalog authentication token source is nil")
	}
	return fetchRealms(ctx, src)
}

// Friends lists the friends' worlds the account can join.
func Friends(ctx context.Context, src oauth2.TokenSource) ([]Friend, error) {
	if src == nil {
		return nil, errors.New("catalog authentication token source is nil")
	}
	xbl, err := newXSAPIClient(ctx, src)
	if err != nil {
		return nil, err
	}
	defer xbl.Close()
	return fetchFriends(ctx, xbl)
}

// Gamertag returns the signed-in account's gamertag.
func Gamertag(ctx context.Context, src oauth2.TokenSource) (string, error) {
	if src == nil {
		return "", errors.New("catalog authentication token source is nil")
	}
	xbl, err := newXSAPIClient(ctx, src)
	if err != nil {
		return "", err
	}
	defer xbl.Close()
	return xbl.UserInfo().GamerTag, nil
}
