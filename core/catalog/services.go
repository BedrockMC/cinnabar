package catalog

import (
	"context"

	"github.com/df-mc/go-xsapi/v2"
	"github.com/hashimthearab/rust-mcbe/core/authcache"
)

// Realms lists the account's Realms with their join targets.
func Realms(ctx context.Context, account *authcache.Account) ([]Realm, error) {
	return fetchRealms(ctx, account)
}

// Friends lists the friends' worlds the account can join.
func Friends(ctx context.Context, account *authcache.Account) ([]Friend, error) {
	xbl, err := newXSAPIClient(ctx, account)
	if err != nil {
		return nil, err
	}
	defer xbl.Close()
	return fetchFriends(ctx, xbl)
}

// Gamertag returns the signed-in account's gamertag.
func Gamertag(ctx context.Context, account *authcache.Account) (string, error) {
	xbl, err := newXSAPIClient(ctx, account)
	if err != nil {
		return "", err
	}
	defer xbl.Close()
	return xbl.UserInfo().GamerTag, nil
}

// XboxClient signs in to Xbox Live with the account; the caller closes it.
func XboxClient(ctx context.Context, account *authcache.Account) (*xsapi.Client, error) {
	return newXSAPIClient(ctx, account)
}
