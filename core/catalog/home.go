package catalog

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/hashimthearab/rust-mcbe/core/authcache"
	"github.com/sandertv/gophertunnel/minecraft/protocol"
	"github.com/sandertv/gophertunnel/minecraft/realms"
	"github.com/sandertv/gophertunnel/minecraft/service"
)

// Home is what the start screen shows from services: messaging surfaces and
// the inbox, the token's treatments, the Realms invite count, live events and
// the rendered persona head. A failed part is named in Errors.
type Home struct {
	Messages     []Message   `json:"messages"`
	Inbox        Inbox       `json:"inbox"`
	Treatments   []string    `json:"treatments"`
	RealmInvites int         `json:"realm_invites"`
	LiveEvents   []LiveEvent `json:"live_events"`
	PersonaHead  Image       `json:"persona_head"`
	Errors       []string    `json:"errors,omitempty"`
	failed       homePart
}

// homePart marks one independently fetched part of Home.
type homePart uint8

const (
	partInvites homePart = 1 << iota
	partTreatments
	partMessages
	partEvents
	partPersona
	partServices = partTreatments | partMessages | partEvents | partPersona
	allHomeParts = partInvites | partServices
)

// Failed reports whether no part of a fetched Home succeeded.
func (h Home) Failed() bool { return h.failed == allHomeParts }

// Refill returns h with its failed parts taken from previous, so a partial refresh keeps the
// last good data.
func (h Home) Refill(previous Home) Home {
	if h.failed&partInvites != 0 {
		h.RealmInvites = previous.RealmInvites
	}
	if h.failed&partTreatments != 0 {
		h.Treatments = previous.Treatments
	}
	if h.failed&partMessages != 0 {
		h.Messages, h.Inbox = previous.Messages, previous.Inbox
	}
	if h.failed&partEvents != 0 {
		h.LiveEvents = previous.LiveEvents
	}
	if h.failed&partPersona != 0 {
		h.PersonaHead = previous.PersonaHead
	}
	return h
}

// HomeImages lists the artwork of home for CacheImages.
func HomeImages(home *Home) []*Image {
	var images []*Image
	for index := range home.Messages {
		for image := range home.Messages[index].Images {
			images = append(images, &home.Messages[index].Images[image].Image)
		}
	}
	for index := range home.LiveEvents {
		images = append(images, &home.LiveEvents[index].Badge, &home.LiveEvents[index].EventImage)
	}
	return append(images, &home.PersonaHead)
}

// Message is one player-messaging message; Surface places it (PlayButton,
// MarketplaceButton, InboxMessage, LoginAnnouncement, ToastNotification, ...).
type Message struct {
	ID         string          `json:"id"`
	InstanceID string          `json:"instance_id"`
	ReportID   string          `json:"report_id,omitempty"`
	Surface    string          `json:"surface"`
	Template   string          `json:"template"`
	Category   string          `json:"category,omitempty"`
	Status     string          `json:"status,omitempty"`
	Received   string          `json:"received,omitempty"`
	Header     string          `json:"header,omitempty"`
	Body       string          `json:"body,omitempty"`
	SubTitle   string          `json:"sub_title,omitempty"`
	Banner     string          `json:"banner,omitempty"`
	Images     []MessageImage  `json:"images"`
	Buttons    []MessageButton `json:"buttons"`
}

// MessageImage is one keyed message image.
type MessageImage struct {
	ID string `json:"id"`
	Image
}

// MessageButton is one keyed message button; Action is external, internal,
// pageid or productid and says how Link opens.
type MessageButton struct {
	ID     string `json:"id"`
	Text   string `json:"text"`
	Link   string `json:"link,omitempty"`
	Action string `json:"action,omitempty"`
}

// Inbox is the inbox summary with per-category counts.
type Inbox struct {
	Total      int             `json:"total"`
	Unread     int             `json:"unread"`
	Categories []InboxCategory `json:"categories"`
}

type InboxCategory struct {
	Type   string `json:"type"`
	Name   string `json:"name"`
	Total  int    `json:"total"`
	Unread int    `json:"unread"`
}

// LiveEvent is a live gathering with the active segment's start-screen UI.
type LiveEvent struct {
	ID                string `json:"id"`
	Title             string `json:"title"`
	Description       string `json:"description,omitempty"`
	StartUnix         int64  `json:"start_unix"`
	EndUnix           int64  `json:"end_unix"`
	RouteToServers    bool   `json:"route_to_servers,omitempty"`
	Address           string `json:"address,omitempty"`
	NetherNetID       string `json:"nethernet_id,omitempty"`
	ButtonText        string `json:"button_text,omitempty"`
	CaptionText       string `json:"caption_text,omitempty"`
	CaptionCountdown  bool   `json:"caption_countdown,omitempty"`
	CaptionBackground string `json:"caption_background,omitempty"`
	CaptionForeground string `json:"caption_foreground,omitempty"`
	HeaderText        string `json:"header_text,omitempty"`
	TitleText         string `json:"title_text,omitempty"`
	BodyText          string `json:"body_text,omitempty"`
	Badge             Image  `json:"badge"`
	EventImage        Image  `json:"event_image"`
}

// MessagingSession carries the messaging session id and continuation across calls.
type MessagingSession struct {
	mu           sync.Mutex
	id           string
	continuation string
}

func (s *MessagingSession) current() (string, string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.id == "" {
		s.id = uuid.NewString()
	}
	return s.id, s.continuation
}

func (s *MessagingSession) advance(continuation string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if continuation != "" {
		s.continuation = continuation
	}
}

const (
	clientPlatform  = "Android"
	clientSub       = "Google"
	maxServiceBytes = 4 * 1024 * 1024
)

// HomeFeed gathers the start screen's service data; the persona head is
// written into artworkDir when the service returns image bytes.
func HomeFeed(ctx context.Context, account *authcache.Account, session *MessagingSession, artworkDir string) (Home, error) {
	home := Home{Messages: []Message{}, Treatments: []string{}, LiveEvents: []LiveEvent{}}
	if account == nil {
		return home, errNoAccount
	}
	fail := func(part homePart, name string, err error) {
		home.failed |= part
		home.Errors = append(home.Errors, name+": "+err.Error())
	}
	if count, err := realmInvites(ctx, account); err != nil {
		fail(partInvites, "Realms invites", err)
	} else {
		home.RealmInvites = count
	}
	err := withServices(ctx, account, func(s *serviceSession) error {
		if token, err := s.tokens.ServiceToken(ctx); err == nil {
			home.Treatments = append(home.Treatments, token.Treatments...)
		} else {
			fail(partTreatments, "Treatments", err)
		}
		if err := s.messages(ctx, session, &home); err != nil {
			fail(partMessages, "Messaging", err)
		}
		if events, err := s.liveEvents(ctx); err != nil {
			fail(partEvents, "Live events", err)
		} else {
			home.LiveEvents = events
		}
		if head, err := s.personaHead(ctx, artworkDir); err != nil {
			fail(partPersona, "Persona", err)
		} else {
			home.PersonaHead = head
		}
		return nil
	})
	if err != nil {
		fail(partServices, "Services", err)
	}
	return home, nil
}

// ReportMessageEvent posts one messaging event (Impression, Click, Dismiss, ...).
func ReportMessageEvent(ctx context.Context, account *authcache.Account, session *MessagingSession, event MessageEvent) error {
	return withServices(ctx, account, func(s *serviceSession) error {
		id, continuation := session.current()
		entry := map[string]any{
			"eventDateTime": time.Now().UTC().Format("2006-01-02T15:04:05.000Z"),
			"eventType":     event.Type,
			"sessionId":     id,
		}
		if event.InstanceID != "" {
			entry["instanceId"] = event.InstanceID
			entry["reportId"] = event.ReportID
		}
		if event.ButtonID != "" {
			entry["buttonId"] = event.ButtonID
		}
		body := map[string]any{"SessionId": id, "continuationToken": continuation, "events": []any{entry}}
		_, err := s.call(ctx, "messaging", http.MethodPost, "/api/v1.0/messages/event", id, body)
		return err
	})
}

// MessageEvent is one messaging report.
type MessageEvent struct {
	Type       string
	InstanceID string
	ReportID   string
	ButtonID   string
}

// serviceSession is one signed-in Minecraft-services session.
type serviceSession struct {
	discovery *service.Discovery
	tokens    service.TokenSource
	xuid      string
	client    *http.Client
}

// serviceURI reads a discovery environment's serviceUri.
func (s *serviceSession) serviceURI(name string) (*url.URL, error) {
	environment, ok := s.discovery.ServiceEnvironments[name]["prod"]
	if !ok {
		return nil, fmt.Errorf("%s is not discovered", name)
	}
	var config struct {
		ServiceURI string `json:"serviceUri"`
	}
	if err := json.Unmarshal(environment, &config); err != nil {
		return nil, fmt.Errorf("decode %s environment: %w", name, err)
	}
	uri, err := url.Parse(config.ServiceURI)
	if err != nil || uri.Scheme != "https" || uri.Host == "" {
		return nil, fmt.Errorf("%s has no https service uri", name)
	}
	return uri, nil
}

// call sends an MCToken-authorized request and returns the raw response body.
func (s *serviceSession) call(ctx context.Context, environment, method, path, sessionID string, body any) ([]byte, error) {
	base, err := s.serviceURI(environment)
	if err != nil {
		return nil, err
	}
	target := base.JoinPath(path)
	if query := strings.SplitN(path, "?", 2); len(query) == 2 {
		target = base.JoinPath(query[0])
		target.RawQuery = query[1]
	}
	var reader io.Reader
	if body != nil {
		encoded, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		reader = bytes.NewReader(encoded)
	}
	req, err := http.NewRequestWithContext(ctx, method, target.String(), reader)
	if err != nil {
		return nil, err
	}
	token, err := s.tokens.ServiceToken(ctx)
	if err != nil {
		return nil, fmt.Errorf("service token: %w", err)
	}
	token.SetAuthHeader(req)
	req.Header.Set("Session-Id", sessionID)
	req.Header.Set("Accept-Language", "en-US")
	req.Header.Set("User-Agent", "libhttpclient/1.0.0.0")
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	return send(s.client, req)
}

func send(client *http.Client, req *http.Request) ([]byte, error) {
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxServiceBytes+1))
	if err != nil {
		return nil, err
	}
	if len(data) > maxServiceBytes {
		return nil, errors.New("response too large")
	}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return nil, fmt.Errorf("HTTP %d", resp.StatusCode)
	}
	return data, nil
}

// withServices hands run a session on the account's shared service token.
func withServices(ctx context.Context, account *authcache.Account, run func(*serviceSession) error) error {
	xbl, err := newXSAPIClient(ctx, account)
	if err != nil {
		return err
	}
	defer xbl.Close()
	discovery, err := service.Default(ctx)
	if err != nil {
		return fmt.Errorf("discover services: %w", err)
	}
	return run(&serviceSession{discovery: discovery, tokens: account, xuid: xbl.UserInfo().XUID, client: http.DefaultClient})
}

func (s *serviceSession) messages(ctx context.Context, session *MessagingSession, home *Home) error {
	id, continuation := session.current()
	data, err := s.call(ctx, "messaging", http.MethodPost, "/api/v1.0/session/refresh", id,
		map[string]string{"sessionId": id, "continuationToken": continuation})
	if err != nil {
		return err
	}
	var envelope struct {
		Result messagingResult `json:"result"`
	}
	if err := json.Unmarshal(data, &envelope); err != nil {
		return fmt.Errorf("decode messages: %w", err)
	}
	session.advance(envelope.Result.ContinuationToken)
	home.Messages, home.Inbox = envelope.Result.flatten()
	return nil
}

type messagingResult struct {
	ContinuationToken string           `json:"continuationToken"`
	Messages          []messageWire    `json:"messages"`
	InboxSummary      inboxSummaryWire `json:"inboxSummary"`
}

type inboxSummaryWire struct {
	Total      int `json:"totalNumberOfMessages"`
	Categories []struct {
		Total        int `json:"totalNumberOfMessages"`
		Unread       int `json:"totalNumberOfUnreadMessages"`
		CategoryInfo struct {
			Type string `json:"type"`
			Name string `json:"name"`
		} `json:"categoryInfo"`
		Messages []messageWire `json:"messages"`
	} `json:"categories"`
}

type messageWire struct {
	ID          string `json:"id"`
	InstanceID  string `json:"instanceId"`
	ReportID    string `json:"reportId"`
	Surface     string `json:"surface"`
	Template    string `json:"template"`
	Category    string `json:"inboxCategory"`
	Status      string `json:"status"`
	Received    string `json:"dateReceived"`
	MessageText struct {
		Header     string `json:"header"`
		Body       string `json:"body"`
		SubTitle   string `json:"subTitle"`
		BannerText string `json:"bannerText"`
	} `json:"messageText"`
	Images map[string]struct {
		URL string `json:"url"`
	} `json:"images"`
	Buttons map[string]struct {
		Text   string `json:"text"`
		Link   string `json:"link"`
		Action string `json:"action"`
	} `json:"buttons"`
}

// flatten keeps well-formed messages (id, surface and template set), top-level
// then per-category, each (id, instance) once, plus the inbox counts.
func (r messagingResult) flatten() ([]Message, Inbox) {
	inbox := Inbox{Total: r.InboxSummary.Total, Categories: []InboxCategory{}}
	wires := append([]messageWire(nil), r.Messages...)
	for _, category := range r.InboxSummary.Categories {
		inbox.Unread += max(category.Unread, 0)
		inbox.Categories = append(inbox.Categories, InboxCategory{
			Type: category.CategoryInfo.Type, Name: category.CategoryInfo.Name,
			Total: category.Total, Unread: category.Unread,
		})
		wires = append(wires, category.Messages...)
	}
	seen := make(map[[2]string]bool)
	messages := []Message{}
	for _, wire := range wires {
		key := [2]string{wire.ID, wire.InstanceID}
		if wire.ID == "" || wire.Surface == "" || wire.Template == "" || seen[key] {
			continue
		}
		seen[key] = true
		message := Message{
			ID: wire.ID, InstanceID: wire.InstanceID, ReportID: wire.ReportID,
			Surface: wire.Surface, Template: wire.Template, Category: wire.Category,
			Status: wire.Status, Received: wire.Received,
			Header: wire.MessageText.Header, Body: wire.MessageText.Body,
			SubTitle: wire.MessageText.SubTitle, Banner: wire.MessageText.BannerText,
			Images: []MessageImage{}, Buttons: []MessageButton{},
		}
		for id, image := range wire.Images {
			if validArtworkURL(image.URL) {
				message.Images = append(message.Images, MessageImage{ID: id, Image: Image{URL: image.URL}})
			}
		}
		for id, button := range wire.Buttons {
			message.Buttons = append(message.Buttons, MessageButton{
				ID: id, Text: button.Text, Link: button.Link, Action: strings.ToLower(button.Action),
			})
		}
		sortMessageParts(&message)
		messages = append(messages, message)
	}
	return messages, inbox
}

func sortMessageParts(message *Message) {
	sortBy(message.Images, func(a, b MessageImage) bool { return a.ID < b.ID })
	sortBy(message.Buttons, func(a, b MessageButton) bool { return a.ID < b.ID })
}

func sortBy[T any](values []T, less func(a, b T) bool) {
	for i := 1; i < len(values); i++ {
		for j := i; j > 0 && less(values[j], values[j-1]); j-- {
			values[j], values[j-1] = values[j-1], values[j]
		}
	}
}

func (s *serviceSession) liveEvents(ctx context.Context) ([]LiveEvent, error) {
	query := url.Values{
		"clientVersion":     {protocol.CurrentVersion},
		"clientPlatform":    {clientPlatform},
		"clientSubPlatform": {clientSub},
	}.Encode()
	data, err := s.call(ctx, "gatherings", http.MethodGet, "/api/v1.0/config/public?"+query, uuid.NewString(), nil)
	if err != nil {
		return nil, err
	}
	return parseLiveEvents(data, time.Now())
}

type gatheringWire struct {
	ID             string `json:"gatheringId"`
	Start          string `json:"startTimeUtc"`
	End            string `json:"endTimeUtc"`
	Title          string `json:"title"`
	Description    string `json:"description"`
	RouteToServers bool   `json:"shouldRouteToServerTab"`
	Venue          struct {
		NetherNetID string          `json:"netherNetId"`
		Address     string          `json:"serverIpAddress"`
		Port        json.RawMessage `json:"serverPort"`
	} `json:"externalVenue"`
	Segments []struct {
		Start string          `json:"startTimeUtc"`
		End   string          `json:"endTimeUtc"`
		UI    gatheringUIWire `json:"ui"`
	} `json:"segments"`
}

type gatheringUIWire struct {
	BadgeImage        string `json:"badgeImage"`
	EventImage        string `json:"eventImage"`
	HeaderText        string `json:"headerText"`
	TitleText         string `json:"titleText"`
	BodyText          string `json:"bodyText"`
	ButtonText        string `json:"startScreenButtonText"`
	CaptionText       string `json:"captionText"`
	CaptionCountdown  bool   `json:"captionIncludesCountdown"`
	CaptionBackground string `json:"captionBackgroundColor"`
	CaptionForeground string `json:"captionForegroundColor"`
}

// parseLiveEvents finds every object carrying a gathering id, wherever the
// response nests it, and keeps those not yet over.
func parseLiveEvents(data []byte, now time.Time) ([]LiveEvent, error) {
	var root any
	if err := json.Unmarshal(data, &root); err != nil {
		return nil, fmt.Errorf("decode gatherings: %w", err)
	}
	var found []map[string]any
	var walk func(value any)
	walk = func(value any) {
		switch value := value.(type) {
		case map[string]any:
			if _, ok := value["gatheringId"]; ok {
				found = append(found, value)
				return
			}
			for _, child := range value {
				walk(child)
			}
		case []any:
			for _, child := range value {
				walk(child)
			}
		}
	}
	walk(root)
	events := []LiveEvent{}
	for _, object := range found {
		encoded, err := json.Marshal(object)
		if err != nil {
			continue
		}
		var wire gatheringWire
		if json.Unmarshal(encoded, &wire) != nil || wire.ID == "" {
			continue
		}
		event := LiveEvent{
			ID: wire.ID, Title: strings.TrimSpace(wire.Title), Description: strings.TrimSpace(wire.Description),
			StartUnix: unixOf(wire.Start), EndUnix: unixOf(wire.End), RouteToServers: wire.RouteToServers,
			NetherNetID: wire.Venue.NetherNetID,
		}
		if port := portOf(wire.Venue.Port); wire.Venue.Address != "" && port > 0 {
			event.Address = wire.Venue.Address + ":" + strconv.Itoa(port)
		}
		if event.EndUnix != 0 && event.EndUnix < now.Unix() {
			continue
		}
		// The segment running now dresses the button; else the first one.
		for index, segment := range wire.Segments {
			start, end := unixOf(segment.Start), unixOf(segment.End)
			if index == 0 || (start <= now.Unix() && (end == 0 || now.Unix() < end)) {
				applySegmentUI(&event, segment.UI)
			}
		}
		events = append(events, event)
	}
	return events, nil
}

func applySegmentUI(event *LiveEvent, ui gatheringUIWire) {
	event.ButtonText, event.CaptionText = ui.ButtonText, ui.CaptionText
	event.CaptionCountdown = ui.CaptionCountdown
	event.CaptionBackground, event.CaptionForeground = ui.CaptionBackground, ui.CaptionForeground
	event.HeaderText, event.TitleText, event.BodyText = ui.HeaderText, ui.TitleText, ui.BodyText
	event.Badge, event.EventImage = Image{}, Image{}
	if validArtworkURL(ui.BadgeImage) {
		event.Badge.URL = ui.BadgeImage
	}
	if validArtworkURL(ui.EventImage) {
		event.EventImage.URL = ui.EventImage
	}
}

func unixOf(value string) int64 {
	parsed, err := time.Parse(time.RFC3339, strings.TrimSpace(value))
	if err != nil {
		return 0
	}
	return parsed.Unix()
}

func portOf(raw json.RawMessage) int {
	text := strings.Trim(strings.TrimSpace(string(raw)), `"`)
	port, err := strconv.Atoi(text)
	if err != nil || port <= 0 || port > 65535 {
		return 0
	}
	return port
}

// personaHead fetches the rendered persona head for the signed-in account.
func (s *serviceSession) personaHead(ctx context.Context, artworkDir string) (Image, error) {
	if s.xuid == "" {
		return Image{}, errors.New("no xuid")
	}
	data, err := s.call(ctx, "persona", http.MethodGet,
		"/api/v1.0/profile/xuid/"+url.PathEscape(s.xuid)+"/image/head", uuid.NewString(), nil)
	if err != nil {
		return Image{}, err
	}
	// The service answers with the image itself, or a JSON body naming its URL.
	if strings.HasPrefix(http.DetectContentType(data), "image/") {
		if artworkDir == "" {
			return Image{}, nil
		}
		if err := os.MkdirAll(artworkDir, 0o700); err != nil {
			return Image{}, err
		}
		path := filepath.Join(artworkDir, "persona-head.img")
		if err := os.WriteFile(path, data, 0o600); err != nil {
			return Image{}, err
		}
		return Image{Path: path}, nil
	}
	var body struct {
		Result struct {
			URL string `json:"url"`
		} `json:"result"`
		URL string `json:"url"`
	}
	if err := json.Unmarshal(data, &body); err != nil {
		return Image{}, fmt.Errorf("decode persona image: %w", err)
	}
	for _, candidate := range []string{body.Result.URL, body.URL} {
		if validArtworkURL(candidate) {
			return Image{URL: candidate}, nil
		}
	}
	return Image{}, errors.New("no persona image")
}

// realmInvites reads the pending Realms invite count through the Realms client.
func realmInvites(ctx context.Context, account *authcache.Account) (int, error) {
	return realms.NewClient(account, nil).PendingInviteCount(ctx)
}
