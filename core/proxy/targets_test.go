package proxy

import (
	"testing"

	"github.com/sandertv/gophertunnel/minecraft/p2p"
)

func TestSelectFriendWorldPrefersFriendsJoinable(t *testing.T) {
	worlds := []p2p.World{
		{OwnerID: "1", Joinability: p2p.JoinabilityInviteOnly, WorldName: "invite"},
		{OwnerID: "2", Joinability: p2p.JoinabilityFriends, WorldName: "other"},
		{OwnerID: "1", Joinability: p2p.JoinabilityFriends, WorldName: "friends"},
	}
	if got := selectFriendWorld(worlds, "1"); got == nil || got.WorldName != "friends" {
		t.Fatalf("selected %+v, want friends world", got)
	}
}

func TestSelectFriendWorldFallsBackToInviteOnlyThenNil(t *testing.T) {
	worlds := []p2p.World{{OwnerID: "1", Joinability: p2p.JoinabilityInviteOnly, WorldName: "invite"}}
	if got := selectFriendWorld(worlds, "1"); got == nil || got.WorldName != "invite" {
		t.Fatalf("selected %+v, want invite-only world", got)
	}
	if got := selectFriendWorld(worlds, "9"); got != nil {
		t.Fatalf("selected %+v for an absent owner", got)
	}
}
