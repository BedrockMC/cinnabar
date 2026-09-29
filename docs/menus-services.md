# Menu service feeds

Which services feed each out-of-game screen, as the vanilla 26.30 client uses them, and what
Cinnabar serves over the control channel (`docs/control-channel.md`). Behaviour comes from the
26.30 reconstruction; hosts are named only where an open-source library (gophertunnel,
go-xsapi, go-playfab) confirms them. ⚑ = seen only in the reconstruction; needs independent
capture before it is implemented.

Tokens: **MCToken** is the Minecraft-services authorization from the discovery `auth`
environment (gophertunnel `minecraft/service`), started from a PlayFab session; **PF** is a
PlayFab session/entity token; **XSTS(rp)** is an Xbox Live token for relying party `rp`.

| Surface | Service (host) | Auth | When | Cinnabar |
| --- | --- | --- | --- | --- |
| Discovery | `client.discovery.minecraft-services.net` | none | startup, cached | `catalog` (gophertunnel) |
| Featured servers (Servers tab) | gatherings, from discovery (open default `gatherings-secondary.franchise.minecraft-services.net`) | MCToken | Servers tab open, MCToken refresh | `featured_servers.v1` |
| Gatherings / live events | gatherings environment | MCToken | periodic after MCToken | `gatherings.v1` (experiences + join); venue/eligibility ⚑ |
| Realms worlds | `bedrock.frontendlegacy.realms.minecraft-services.net` `/worlds` | XSTS(`https://pocket.realms.minecraft.net/`) | Realms tab, play screen | `realms_list.v1` (owner, players, expiry) |
| Realms invites count / lists | Realms ⚑ | XSTS(realms) | start screen badge | not served |
| Friend worlds | `sessiondirectory.xboxlive.com` (MPSD activity handles) | XSTS(`http://xboxlive.com`) | Friends tab, interval | `friends_list.v1` |
| Friends / presence / gamerpic | `peoplehub.xboxlive.com`, `social.xboxlive.com`, RTA | XSTS(xboxlive) | play screen, live | `profile.v1` (self only) |
| Profile gamerpic (vanilla) | Xbox profile settings ⚑ | XSTS(xboxlive) | start screen | peoplehub picture instead |
| Persona appearance | persona environment ⚑ | MCToken | start/profile | not served |
| Announcements, inbox, Play/Store tile art, toasts | player-messaging environment ⚑ | MCToken + session | after MCToken, timer | not served (no open endpoint) |
| Store layouts, offers, treatment content | store environment ⚑ + PlayFab catalog (`<titleId>.playfabapi.com`) | PF / MCToken treatments | about daily | not served |

Server player counts and MOTDs on the Servers tab come from a RakNet ping, not a service.
The MCToken response carries the treatment (feature-flag) list.

## Screen bindings each feed populates

- **Start screen** (`start.start_screen`): `#gamertag_label`, `#playername`,
  `#gamertag_pic_and_label_visible` (profile); `#gathering_*` button and countdown
  (gatherings); `#realms_notification_count` (invites, not served); Play/Store art
  (`#play_button_art_*`, `#store_button_art_*`) and `#unread_notification_icon` (messaging, not
  served). The home news carousel of newer clients is OreUI, not part of `ui/*.json`.
- **Play screen** (`play.play_screen`): `third_party_server_network_worlds` items
  (`#third_party_server_name`, `#third_party_server_message`,
  `#third_party_server_logo_texture_path`), the selected server's info panel
  (`#info_third_party_server_name`, `#description_label`, `#news_text`, `server_games_collection`,
  `server_screenshot_collection`); `personal_realms` / `friends_realms`
  (`#realms_world_player_count`, expiry); `friends_network_worlds` (`#network_world_header`,
  `#network_world_details`, `#network_world_player_count`); `servers_network_worlds` (saved).
- **Profile**: the 26.30 profile/character screens are OreUI; no `ui/*.json` screen exists.

Artwork is fetched by the core over HTTPS into a bounded per-run cache and drawn from local
files (`catalog.CacheImages`).
