package catalog

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/df-mc/go-playfab/v2"
	playfabcatalog "github.com/df-mc/go-playfab/v2/catalog"
	"github.com/df-mc/go-xsapi/v2"
	"github.com/sandertv/gophertunnel/minecraft/service"
	"github.com/sandertv/gophertunnel/minecraft/service/gatherings"
	"golang.org/x/oauth2"
)

// FeaturedServer is a featured server with the details the play screen's
// server info panel shows. Image fields are HTTPS URLs; paths are filled in by
// a caller that caches the artwork.
type FeaturedServer struct {
	Name        string   `json:"name"`
	Address     string   `json:"address"`
	Caption     string   `json:"caption"`
	Description string   `json:"description,omitempty"`
	NewsTitle   string   `json:"news_title,omitempty"`
	News        string   `json:"news,omitempty"`
	Logo        Image    `json:"logo"`
	Screenshots []Image  `json:"screenshots"`
	Games       []Game   `json:"games"`
	Tags        []string `json:"tags,omitempty"`
}

// Game is one activity a featured server or gathering advertises.
type Game struct {
	Title       string `json:"title"`
	Subtitle    string `json:"subtitle,omitempty"`
	Description string `json:"description,omitempty"`
	Image       Image  `json:"image"`
}

// Image is remote HTTPS artwork plus its locally cached copy, when one exists.
type Image struct {
	URL  string `json:"url,omitempty"`
	Path string `json:"path,omitempty"`
}

// Gathering is a community experience; Address is empty when joining it
// could not be resolved.
type Gathering struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Caption     string `json:"caption"`
	Description string `json:"description,omitempty"`
	Creator     string `json:"creator,omitempty"`
	Address     string `json:"address,omitempty"`
	Image       Image  `json:"image"`
	StartUnix   int64  `json:"start_unix,omitempty"`
	EndUnix     int64  `json:"end_unix,omitempty"`
}

// Profile is the signed-in account as the start and profile screens show it.
type Profile struct {
	Gamertag     string `json:"gamertag"`
	XUID         string `json:"xuid"`
	Gamerpic     Image  `json:"gamerpic"`
	RealName     string `json:"real_name,omitempty"`
	PresenceText string `json:"presence_text,omitempty"`
	Gamerscore   int64  `json:"gamerscore"`
	Friends      int    `json:"friends"`
	Followers    int    `json:"followers"`
}

// FeaturedServers lists the featured servers from the gatherings service.
func FeaturedServers(ctx context.Context, src oauth2.TokenSource) ([]FeaturedServer, error) {
	var result []FeaturedServer
	err := withGatherings(ctx, src, func(_ *xsapi.Client, client *gatherings.Client) error {
		values, err := client.FeaturedServers(ctx)
		if err != nil {
			return err
		}
		result = featuredServers(values)
		return nil
	}, nil)
	return result, err
}

// Gatherings lists the community experiences with their join addresses.
func Gatherings(ctx context.Context, src oauth2.TokenSource) ([]Gathering, error) {
	var result []Gathering
	err := withGatherings(ctx, src, func(_ *xsapi.Client, client *gatherings.Client) error {
		values, err := client.Experiences(ctx)
		if err != nil {
			return err
		}
		result = make([]Gathering, 0, len(values))
		for _, experience := range values {
			if experience == nil || !experience.Valid() {
				continue
			}
			entry := gathering(experience)
			joinContext, cancel := context.WithTimeout(ctx, 5*time.Second)
			if address, err := experience.Join(joinContext); err == nil && address != nil && address.String() != ":0" {
				entry.Address = address.String()
			}
			cancel()
			result = append(result, entry)
		}
		return nil
	}, nil)
	return result, err
}

// AccountProfile returns the signed-in gamertag, XUID and gamerpic; a missing
// gamerpic is not an error.
func AccountProfile(ctx context.Context, src oauth2.TokenSource) (Profile, error) {
	if src == nil {
		return Profile{}, errors.New("catalog authentication token source is nil")
	}
	xbl, err := newXSAPIClient(ctx, src)
	if err != nil {
		return Profile{}, err
	}
	defer xbl.Close()
	info := xbl.UserInfo()
	profile := Profile{Gamertag: info.GamerTag, XUID: info.XUID}
	social := xbl.Social()
	if user, err := social.UserByXUID(ctx, info.XUID); err == nil {
		if validArtworkURL(user.DisplayPictureRawURL) {
			profile.Gamerpic.URL = user.DisplayPictureRawURL
		}
		if profile.Gamertag == "" {
			profile.Gamertag = strings.TrimSpace(user.GamerTag)
		}
		profile.RealName = strings.TrimSpace(user.RealName)
		profile.PresenceText = strings.TrimSpace(user.PresenceText)
		if score, err := user.GamerScore.Int64(); err == nil && score > 0 {
			profile.Gamerscore = score
		}
	}
	if friends, err := social.Friends(ctx); err == nil {
		profile.Friends = len(friends)
	}
	if followers, err := social.Followers(ctx); err == nil {
		profile.Followers = len(followers)
	}
	return profile, nil
}

// withGatherings signs in to Xbox Live, PlayFab and the Minecraft-services
// auth environment, then hands a gatherings client and/or a service session
// to whichever callbacks are set.
func withGatherings(
	ctx context.Context,
	src oauth2.TokenSource,
	runGatherings func(*xsapi.Client, *gatherings.Client) error,
	runServices func(*serviceSession) error,
) error {
	if src == nil {
		return errors.New("catalog authentication token source is nil")
	}
	xbl, err := newXSAPIClient(ctx, src)
	if err != nil {
		return err
	}
	defer xbl.Close()
	discovery, err := service.Default(ctx)
	if err != nil {
		return fmt.Errorf("discover services: %w", err)
	}
	env := new(service.AuthorizationEnvironment)
	if err := discovery.Environment(env); err != nil {
		return fmt.Errorf("resolve services: %w", err)
	}
	session, err := playfab.LoginWithXbox(ctx, env.PlayFabTitleID, xbl, playfab.ClientConfig{CreateAccount: true})
	if err != nil {
		return fmt.Errorf("PlayFab login: %w", err)
	}
	defer session.Close()
	tokens := env.TokenSource(session, service.TokenConfig{})
	if runGatherings != nil {
		if err := runGatherings(xbl, gatherings.NewClient(tokens)); err != nil {
			return err
		}
	}
	if runServices != nil {
		return runServices(&serviceSession{
			discovery: discovery,
			tokens:    tokens,
			xuid:      xbl.UserInfo().XUID,
			client:    http.DefaultClient,
		})
	}
	return nil
}

func featuredServers(values []*gatherings.FeaturedServer) []FeaturedServer {
	result := make([]FeaturedServer, 0, len(values))
	for _, server := range values {
		if server == nil || !server.Valid() {
			continue
		}
		result = append(result, FeaturedServer{
			Name:        displayName(server.Item.Title.Neutral(), server.CreatorName, "Featured server"),
			Address:     server.Address(),
			Caption:     firstGameCaption(server.AvailableGames, "Featured server"),
			Description: strings.TrimSpace(server.Item.Description.Neutral()),
			NewsTitle:   strings.TrimSpace(server.NewsTitle),
			News:        strings.TrimSpace(server.News),
			Logo:        Image{URL: artworkURL(server.Item, nil)},
			Screenshots: screenshots(server.Item),
			Games:       games(server.Item, server.AvailableGames),
			Tags:        server.Item.Tags,
		})
	}
	return result
}

func gathering(experience *gatherings.Experience) Gathering {
	entry := Gathering{
		ID:          experience.ID.String(),
		Name:        displayName(experience.Item.Title.Neutral(), experience.CreatorName, "Gathering"),
		Caption:     firstGameCaption(experience.AvailableGames, "Community gathering"),
		Description: strings.TrimSpace(experience.Item.Description.Neutral()),
		Creator:     strings.TrimSpace(experience.CreatorName),
		Image:       Image{URL: artworkURL(experience.Item, experience.AvailableGames)},
	}
	if !experience.Item.StartDate.IsZero() {
		entry.StartUnix = experience.Item.StartDate.Unix()
	}
	if !experience.Item.EndDate.IsZero() {
		entry.EndUnix = experience.Item.EndDate.Unix()
	}
	return entry
}

func screenshots(item playfabcatalog.Item) []Image {
	result := []Image{}
	for _, image := range item.Images {
		if strings.EqualFold(image.Type, playfabcatalog.ImageTypeScreenshot) && validArtworkURL(image.URL) {
			result = append(result, Image{URL: image.URL})
		}
	}
	return result
}

func games(item playfabcatalog.Item, values []gatherings.AvailableGame) []Game {
	result := make([]Game, 0, len(values))
	for _, value := range values {
		game := Game{
			Title:       strings.TrimSpace(value.Title),
			Subtitle:    strings.TrimSpace(value.Subtitle),
			Description: strings.TrimSpace(value.Description),
		}
		for _, image := range item.Images {
			if value.ImageTag != "" && image.Tag == value.ImageTag && validArtworkURL(image.URL) {
				game.Image.URL = image.URL
				break
			}
		}
		if game.Title != "" || game.Subtitle != "" {
			result = append(result, game)
		}
	}
	return result
}

// maxCachedArtwork bounds the artwork directory; the least recently used files go first.
const maxCachedArtwork = 256

// CacheImages downloads each image into directory and fills its path; a
// failed download leaves the path empty.
func CacheImages(ctx context.Context, directory string, images []*Image) {
	if len(images) == 0 || os.MkdirAll(directory, 0o700) != nil {
		return
	}
	for _, image := range images {
		if image == nil || image.URL == "" {
			continue
		}
		if path, err := cacheArtworkFile(ctx, directory, image.URL); err == nil {
			image.Path = path
		}
	}
	pruneArtwork(directory, maxCachedArtwork)
}

// FeaturedImages lists the artwork of servers for CacheImages.
func FeaturedImages(servers []FeaturedServer) []*Image {
	var images []*Image
	for index := range servers {
		server := &servers[index]
		images = append(images, &server.Logo)
		for shot := range server.Screenshots {
			images = append(images, &server.Screenshots[shot])
		}
		for game := range server.Games {
			images = append(images, &server.Games[game].Image)
		}
	}
	return images
}

// GatheringImages lists the artwork of gatherings for CacheImages.
func GatheringImages(gatherings []Gathering) []*Image {
	images := make([]*Image, 0, len(gatherings))
	for index := range gatherings {
		images = append(images, &gatherings[index].Image)
	}
	return images
}

func pruneArtwork(directory string, keep int) {
	entries, err := os.ReadDir(directory)
	if err != nil || len(entries) <= keep {
		return
	}
	type aged struct {
		path     string
		modified time.Time
	}
	files := make([]aged, 0, len(entries))
	for _, entry := range entries {
		if info, err := entry.Info(); err == nil && info.Mode().IsRegular() {
			files = append(files, aged{filepath.Join(directory, entry.Name()), info.ModTime()})
		}
	}
	sort.Slice(files, func(i, j int) bool { return files[i].modified.Before(files[j].modified) })
	for len(files) > keep {
		_ = os.Remove(files[0].path)
		files = files[1:]
	}
}
