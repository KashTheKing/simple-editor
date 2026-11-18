# Platform icons for the device previews

The icons the Device Emulator and the social guides draw: the platforms' own icons, copied from their own
web pages, plus Google's Material Symbols for the Android status bar. Each `<platform>/<icon>.svg` is the SVG
the page itself puts in its DOM (ids, classes and labels stripped, nothing redrawn).
`scripts/platform-icons.sh` rasterises them into `assets/social_icons.png` (white on transparent, logos in
colour, 64 px in a 72 px cell); `src/ui/social/icons.rs` embeds that atlas, so the app needs no SVG renderer.
`atlas.txt` is the atlas order: one `<platform>/<icon> [cells wide]` per line. Run the script again after
adding or replacing an SVG.

Collected on 2026-11-17 in Chrome (desktop, dark theme, 1024 x 605 viewport) from these pages:

| Folder | Platform | Taken from |
|---|---|---|
| `yt/` | YouTube (watch page, player, settings menu, top bar, comments) | `https://www.youtube.com/watch?v=...` (a RoDev video and a Rick Astley video for the captions-on icon), the player's `.ytp-chrome-bottom` buttons in each state (playing, paused, muted, low volume, theatre, captions on / off), its settings menu, `#masthead`, `#above-the-fold`, `ytd-comments` |
| `ys/` | YouTube Shorts | `https://www.youtube.com/shorts/...`, the `ytd-shorts` player |
| `tt/` | TikTok | `https://www.tiktok.com/foryou`: the action column, the player bar, the side navigation and the comments panel |
| `ig/` | Instagram | `https://www.instagram.com/reels/` and a post page `https://www.instagram.com/p/...` (logged out) |
| `x/` | X | `https://x.com/search?q=...%20filter%3Anative_video`: the post's action row and the video player; the action icons (reply, repost, like, views, bookmark, share) from a post page's `<symbol id="icon-*">` sprite definitions (2026-11-18) |
| `android/` | Android status bar | Material Symbols Rounded, filled (`google/material-design-icons`, `symbols/web/<name>/materialsymbolsrounded/<name>_fill1_24px.svg`), Apache License 2.0 |

Drawn in code rather than copied (no page carries them, or they are simple shapes the apps draw the same
way): the iOS status bar (signal bars, Wi-Fi fan, battery), the Dynamic Island, notch and punch hole, the
Samsung three-button bar, the browser window, TikTok's create button and spinning record, YouTube's
verified tick and the avatar placeholder. They follow screenshots of the real screens.

The icons and logos are the property of their owners (YouTube: Google LLC; TikTok: ByteDance Ltd.;
Instagram: Meta Platforms, Inc.; X: X Corp.). Simple Editor uses them only to preview how a video will look
on each platform; it is not affiliated with or endorsed by any of them.
