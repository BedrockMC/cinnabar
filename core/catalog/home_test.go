package catalog

import (
	"encoding/json"
	"testing"
	"time"
)

// Authored to the reconstruction's field names; not a captured payload.
const messagingFixture = `{"result":{
	"continuationToken":"c2",
	"messages":[
		{"id":"m1","instanceId":"i1","surface":"PlayButton","template":"ImageTile",
		 "messageText":{"header":"New!"},"images":{"tile":{"url":"https://cdn.test/t.png"}},
		 "buttons":{"go":{"text":"Play","link":"/play","action":"Internal"}}},
		{"id":"","surface":"PlayButton","template":"x"},
		{"id":"m2","surface":"ToastNotification"}
	],
	"inboxSummary":{"totalNumberOfMessages":3,"categories":[
		{"totalNumberOfMessages":3,"totalNumberOfUnreadMessages":2,
		 "categoryInfo":{"type":"News","name":"News"},
		 "messages":[{"id":"m1","instanceId":"i1","surface":"PlayButton","template":"ImageTile"},
		             {"id":"m3","instanceId":"i3","surface":"InboxMessage","template":"Text","status":"Unread"}]}
	]}
}}`

func TestMessagesKeepWellFormedEntriesOnce(t *testing.T) {
	var envelope struct {
		Result messagingResult `json:"result"`
	}
	if err := json.Unmarshal([]byte(messagingFixture), &envelope); err != nil {
		t.Fatal(err)
	}
	messages, inbox := envelope.Result.flatten()
	if len(messages) != 2 || messages[0].ID != "m1" || messages[1].Surface != "InboxMessage" {
		t.Fatalf("messages = %+v", messages)
	}
	if messages[0].Buttons[0].Action != "internal" || messages[0].Images[0].URL != "https://cdn.test/t.png" {
		t.Fatalf("parts = %+v", messages[0])
	}
	if inbox.Total != 3 || inbox.Unread != 2 || inbox.Categories[0].Type != "News" {
		t.Fatalf("inbox = %+v", inbox)
	}
}

func TestLiveEventsAreFoundAnywhereAndEndedOnesDropped(t *testing.T) {
	data := []byte(`{"result":{"gatherings":[
		{"gatheringId":"g1","title":"Live","startTimeUtc":"2026-09-01T00:00:00Z","endTimeUtc":"2026-12-01T00:00:00Z",
		 "externalVenue":{"serverIpAddress":"1.2.3.4","serverPort":"19132"},
		 "segments":[{"startTimeUtc":"2026-09-01T00:00:00Z","ui":{"startScreenButtonText":"Watch",
		   "captionText":"Live now","captionIncludesCountdown":true,"badgeImage":"https://cdn.test/b.png"}}]},
		{"gatheringId":"old","endTimeUtc":"2020-01-01T00:00:00Z"}
	]}}`)
	now := time.Date(2026, 9, 29, 0, 0, 0, 0, time.UTC)
	events, err := parseLiveEvents(data, now)
	if err != nil || len(events) != 1 {
		t.Fatalf("events = %+v, err = %v", events, err)
	}
	event := events[0]
	if event.Address != "1.2.3.4:19132" || event.ButtonText != "Watch" || !event.CaptionCountdown || event.Badge.URL == "" {
		t.Fatalf("event = %+v", event)
	}
}

func TestPortsReadFromNumbersOrStrings(t *testing.T) {
	for raw, want := range map[string]int{`19132`: 19132, `"19133"`: 19133, `0`: 0, `"x"`: 0} {
		if got := portOf(json.RawMessage(raw)); got != want {
			t.Fatalf("portOf(%s) = %d", raw, got)
		}
	}
}
