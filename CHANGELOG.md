# Changelog

## Unreleased

### Library
- **Linked bins: Import folder as bin is instant now.** A folder comes in as a bin that points at it: nothing is
  copied, scanned or probed up front, its sub-folders show as nested bins, and each bin lists its files when you
  open it (durations and thumbnails fill in for what is on screen). New or removed files show up by themselves. A
  file becomes a project asset the moment you use it (drag it to the timeline, add it from its menu). The project
  saves only the folder. Right-click the bin for Refresh, Relink…, Convert to regular bin (the old import every
  file) and Unlink; a folder that is missing shows as offline. `media.import_folder` links by default
  (`linked: false` imports every file), and `media.linked_bin` lists, refreshes, relinks, converts and unlinks.
- **Library filters are easier to see and undo**: every active filter shows as a chip under the search box (its ✕ drops just that one) with **Clear filter** beside them, and the Filter button lights up while any filter is on. Smart Bins are their own section, with **Save filter…** in its header.
- **Shift+click a bin’s arrow** to open or close it together with every bin inside it; on a linked bin it keeps opening the sub-folders as they are listed.

### Music, playlists and Burn to Disc
- **Tags and cover art for songs and videos**: click a song or video in the Library and the Inspector shows its Tags (title, artist, album, album artist, genre, year, track, disc, comment, lyrics; for video: title, description, genre, rating) and the cover / thumbnail (from a file, from the Source monitor's frame, or removed). Edit several files at once (differing fields show Mixed). Save writes into the files without re-encoding, through a temp file and a `.bak` kept until it worked. A new **Playlists** panel (beside the Media Browser on the Media page): drag songs in from the Library, reorder with the grip or Alt+Up/Down, total length, gap or crossfade, play it in the Source monitor, put it on the timeline as a mix, export `.m3u8`, or **Make visualizer video** (each song with a linked visualizer of the chosen style, its cover art and a title card). **Burn to Disc**: an audio CD (CD audio tracks, gaps, CD-Text, 74 / 80 min check) or a data disc (ISO 9660 + Joliet + UDF, playlists with their `.m3u8`) to a drive (disc check, erase, speed, verify, test only, eject) or to a disc image (`.iso`, `.bin` + `.cue`) with no drive; it runs as a job you can cancel. No video DVD (it needs DVD authoring). Tools: `playlist.*`, `media.tags_get` / `media.tags_set`, `disc.drives` / `disc.burn`.

## 1.3.1

### Export size
- **Exports are about 3x smaller**: the graphics-card encoder now uses lookahead, B-frames and adaptive quality, its quality scale is matched to x264's, and the default quality is a little lower (a 2:12 720p60 visualizer went from 267 MB to ~86 MB). Quick Export uses the same default instead of Max quality, a keyframe lands every 2 s, and the size in the Export output bar is measured from short samples of your timeline. New **Target size** (e.g. 50 MB) aims the file at a size instead of a quality.
### Pages
- **The Cut page is now the Plan page** (Alt+2, opens with the Select tool): Planner, Moodboard, Markers, Subtitles, Voiceover, Transcript and Clips beside Source | Preview and the Inspector, a small Library | Media Browser beside Timeline | Beat Aligner. New default layouts for Edit (Clips beside the Inspector), Color (a big Preview beside Inspector | Nodes | Gallery, Clips | Timeline across the middle, Color Wheels · Curves · Scopes below) and Audio (Library | Media Browser | Gallery · Preview · Voiceover | Transcript | Subtitles · Inspector over Timeline | Beat Aligner · Mixer). Saved page layouts are kept: Window ▸ Layout ▸ Reset Page Layout gets the new ones. **File ▸ Open Project…** (Ctrl+Shift+O) is now in the File menu.
### Audio Only and visualizers
- **Audio Only** (Preview right-click, Playback menu, Ctrl+P, `preview.audio_only`): the Preview plays the timeline's sound with a visualizer of the whole mix instead of the picture (no video decoded), with an Audio Only badge; saved per project, export unaffected. The Source monitor has it too (right-click ▸ Audio Only). **Visualizers are layers of their own**: Gallery ▸ Audio ▸ Visualizers (click = at the playhead, drag onto a video track) adds one that reacts to everything the timeline plays; Inspector: Type, Import shader…, Source (its song or Timeline mix) and a **Frequency bands** curve editor (drag a point per band) with presets and beat source. Also: Ctrl+Shift-drop a song from the Library / Media Browser to add it with a visualizer, and Add Audio Visualizer at the top of a song's right-click menu. Switching between the Source and Preview tabs stops the one hidden (side by side, both play).
### Inspector
- **Compact Inspector properties**: a visual clip shows Transform and Blend and fades (Opacity there) instead of one Properties fold; every keyframable property in the Inspector (Transform, effects, text, shapes, masks, visualizer camera, HTML parameters) is one row of a fixed-width name, the value and ◀ ◆ ▶ right after it, so long names and numbers never move the controls. Scale is Scale X / Scale Y, linked by default (editing one sets the other); Position X / Y are not linked by default; the link sits between the two rows. Speed is one row, in Transform (visual clips) or Audio (audio clips), with no separate Speed, Sound or Speed folds, a video clip shows its linked audio's Volume, Pan and the rest, several text clips edit together (Mixed where they differ), and the clip menu reads Hide Animations while the Animator is shown.
### Assistant scripts and the library
- **The Assistant can manage the whole project with Luau scripts**: `scripts.eval` runs a script straight away against the project and its library (tracks, clips, effects, keyframes, markers, subtitles, sequences, assets, bins, linked folders) as one undo step named after its purpose, undone entirely if it fails. It is sandboxed to the project and library (no files, exports, settings or network), so it needs no Allow; output comes back capped. **Compact library tools**: `media.summary`, filters and fields on `media.list`, `media.bulk_set` and `library.bins`, so a library of hundreds of assets is organised in a few calls. New built-in skill **/organize-library** sorts the library into Footage / SFX / Music / Voiceover / Images / Graphics / Unused bins and tags everything, never deleting.

### Beat Aligner
- **New Beat Aligner panel** (the tab beside the Timeline on the Audio page, a tab on the Plan page, anywhere from a tab bar's +): pick a song, Detect finds the BPM, beat grid and downbeats (from the cached song analysis) under a bar ruler over a big waveform; fix it with BPM, ½ / ×2, Tap tempo, offset and beats per bar. Tap Space, B or M while it plays to drop marks (calibrated latency, optional snap to the beat with a strength), or let Continuous / Fill grid mark the grid. Drag marks (snapping to beats or sub-beats), select stretches into named, coloured sections (Intro, Verse, Chorus, Drop ...) and drag their chips to move or resize them. Then turn them into timeline markers, cut the picture at the marks, put a Library sound on each mark, snap clip edges to them or apply a Zoom Punch / Shake / any Animator block on each beat; export and import CSV, Resolve marker EDL and YouTube chapters. Voice mode marks a voiceover's phrases and pauses instead. Tools: `beats.*`.

### Edit templates
- **Four beat-synced edit templates** in Gallery ▸ Templates: **Phonk Synced Edit**, **Velocity Edit**, **Hype Montage** and **Meme Cut**. Fill the Song slot and your clips (Use selected media takes the Media Library's selection, or the clips selected on the timeline; fewer clips than shots are used again, each time from a different part) and the template finds the song's tempo and drop, retimes every cut onto its beats and cuts the song so the template's drop lands on the song's. Phonk: the song is muffled by a low-pass until the drop, which opens it, slows the shot down grey and red under a **wasted** card with the real sound (its sound slot finds a file named "wasted" in your SFX or Music folders, lined up on its hit), then a flash into fast cuts. Every shot keeps moving: push-ins, zoom punches, shakes, speed ramps or freeze-frames, joined by white flashes, the new **Zoom Momentum** shader transition (also in the online library) or whip pans. All native clips, blocks, effects and transitions.
- Templates can now carry sound slots (a song or a sound effect, aligned on its hit), mixer buses with keyframed filters (the Template Builder keeps a clip's bus and its automation) and a beat grid; `templates.use` takes `drop` to override the detected one.

### Timeline waveforms
- **Clearer audio waveforms on the timeline**: Simple (the default) draws loudness on a dB scale, so quiet speech
  shows and loud music no longer fills the clip, with the peaks as a lighter outline around it. **Detailed** colours
  each moment by its bass (orange), mids (green) and highs (blue), marks hits with ticks, and shows a spectrogram
  heat map on tall tracks. Switch in the timeline ruler's View menu, Settings ▸ General or the palette
  ("Waveforms: Simple / Detailed"); a track's right-click ▸ Waveform overrides it. Waveforms are re-analysed once.
### Export panel redesign
- **A simpler, colour-coded Export panel**: preset cards with icons on top, then Video (blue), Audio (green) and
  Advanced (amber, closed at first) sections that remember whether they are open, and an output bar pinned to the
  bottom with the size · frame rate · length · estimated size, Speed, the file name and folder, and **Export…** /
  **Add to Queue**. Explanations moved into hover tips; nothing was removed. Two built-in cards always come first:
  **Project settings — Max quality** and **Fast preview** (project size and frame rate, fastest encode). The
  platform cards show the real YouTube, Shorts, Instagram and TikTok marks (Simple Icons, CC0). **Your own
  presets**: "Save current settings…" (name, icon, colour), right-click to duplicate, rename, update or delete;
  Quick Export and the new `export.preset_save` / `preset_apply` / `preset_rename` / `preset_delete` tools use them too.
- **Calmer Export panel, more in it**: the sections now look like the Inspector's (no colour bars), the
  selected preset gets the normal selection highlight, platform presets are small square tiles and the brand marks
  follow the text colour. **Frame rate is editable** (Project, 24, 25, 30, 50, 60 or Custom; presets can set it)
  and the export resamples the timeline to it. The **project settings** (name, format, resolution, frame rate,
  background, captions burn-in) are in the Export panel, so the Export page no longer shows the Inspector (a saved
  Export layout is kept: Window ▸ Layout ▸ Reset Page Layout gets the new one). The built-in best-quality
  preset is now just **Max quality**. **Ctrl+P `>Export`** lists **Quick export**: pick the file, it exports with
  the Export panel's settings. **Cancel stops an export at once**: ffmpeg is stopped immediately, the partial file
  is deleted and the job shows Cancelled straight away.
### Panels and screens
- **Voiceover, Transcript and Rendering are panels now** instead of pop-up windows: dock them, tab them, undock
  them, open them from the Window menu or Ctrl+P. Voiceover has the input, a live level meter, Record / Stop,
  punch-in at the playhead, Retake and a list of your takes; Transcript holds transcription, the words and
  captions from the transcript; Rendering shows the running export (and Export Frame) and comes up when an export
  starts. **A real Audio page**: music (Library and the Gallery's Audio side), a small Preview, Mixer / Voiceover /
  Transcript, the Inspector and the Timeline; a saved Audio layout is kept (Window ▸ Layout ▸ Reset Page Layout gets
  the new one). **2- and 3-screen mode**: with more than one monitor the editor asks "Use 2-screen mode?" (also
  Window ▸ Screens… and Settings ▸ General ▸ Layout): the Timeline gets its own window on the second screen, the
  Preview the third. Each page keeps an arrangement per screen count; move any panel with its tab's right-click ▸
  Move to window, or drag a tab out of the window onto another screen. A window whose monitor is gone folds back
  into the main one.

### Quick Export
- **A Quick Export button in the top bar** (top right, beside Jobs; also File ▸ Export ▸ Quick Export… and the
  palette): a Save As dialog with a file type list (MP4, MOV, MKV, WebM, GIF, WAV, MP3), then the export runs as a
  job with your current export settings, or the project's own size, frame rate and quality, and opens in your media
  player when done. For a video you opened directly (one clip, maybe with text or captions on top) the dialog lands
  on that file, and saving asks "Replace <name>?" first; the original stays as `<name>.bak` until the new file is in
  place. Other quick exports remember their folder and type per project.
### Ctrl+P: go to anything
- **The command palette is now Ctrl+P and goes to things by name**: clips on any timeline (nested sequences too),
  timelines, Library media and bins, markers and comments, pages, panels and Gallery cards, grouped by type with
  their track and time or folder. Enter goes there (jump and select, open, reveal); Alt+Enter adds media at the
  playhead or applies a Gallery card. `>` runs commands and on/off settings, `/ask` and `/<skill>` talk to the
  assistant (the only way to it from the palette). Ctrl+K still opens it while nothing else uses Ctrl+K, and a key
  you bound to Ctrl+P yourself keeps it.

### Gallery cards
- **Gallery: a Video | Audio switch with audio presets, and pictures for every title and template.** The Audio side
  has Animate (fades, swell, duck, tremolo, pan sweeps), Transitions (equal-power / linear / exponential crossfades,
  a dip, J- and L-cuts that roll only the sound), Effects (rooms, echoes, telephone, radio, bass boost, de-ess,
  compressor, limiter, gate, pitch up / down, filter sweeps, each a Mixer bus you can edit) and Sounds; cards show
  their curve or waveform, and resting on one plays it on the selected clip. The side is remembered, and selecting
  an audio clip on the timeline opens it. Titles and Templates show a rendered picture (cached), else a type symbol.
- **Skill cards are no longer empty tiles**: each skill is a document card (a .md page tinted by its category, with
  the category's symbol: cut, captions, audio, colour, export, style, converter) with its description beside it and
  a readable title under it; a skill header can name its own picture with `thumb:`.
- Library items without a thumbnail (HTML, icons, effects, transitions, LUTs) show their type symbol, a sound shows
  a waveform symbol while its waveform loads, and card names stay on one line with an ellipsis.

### Assistant chat
- **The Assistant is a chat app now**: a collapsible sidebar of chats (New chat, search over every chat, pin, rename,
  save, delete), user and reply bubbles with Markdown, code blocks with a Copy button, tool steps folded, and a composer
  that grows with the text. Opening the pane starts a new chat with the cursor in the composer; the agent, Tasks, Skills
  and import / export moved into the `⋯` menu, the model and effort pickers sit small beside Send.
- **Copy gives clean text.** Right-click a message: Copy text (no markup), Copy as Markdown, Copy attachments, Retry,
  Edit and resend, Delete. Ctrl+C on text selected in a message copies that text - it no longer also copied the
  selected clips' JSON over it.
- **Attach anything**: the paperclip, a drop on the pane, or Ctrl+V of a picture or of files copied in Explorer. Click a
  thumbnail to preview it; right-click it to Copy (the picture itself) or Save as.
- **Capture the editor into a message**: the camera menu takes the current frame, a screenshot of any panel, an area you
  drag anywhere in the window, or the timeline range (a picture plus its clips and tracks); every pane tab's right-click
  menu has Attach to Assistant. New tool: `assistant.capture`.

### AI enhancer
- **AI Upscale is no longer starved on a busy PC**: the upscaler runs at normal priority and steps down only while
  the timeline or Source monitor plays; the result is encoded on the graphics card (NVENC / QSV / AMF, x264 fallback).
  A short upscale with every core busy went from over 30 minutes to about 2.5 minutes.

### Fixes: playback
- **No more black Preview frame at a cut**: with two clips touching at a cut that is off the frame grid (after a
  ripple, trim or transcript cut), a paused playhead just after the cut, or playback through it, showed black on
  the GPU preview. The preview drew each frame at the clock's time while its layers were decoded for the frame's
  own grid time, so the clip it looked for was not there. Every frame is now drawn at the instant it was decoded
  for: the outgoing clip up to the cut's frame, the incoming one from its first frame on, the same picture export
  makes. Clip edges are not snapped to the grid.

### Fixes: HTML layers
- **"Edge failed to start" on a busy PC**: 1.3.0 started Edge at below-normal priority, so while a build, an
  export or proxies kept every core busy it wrote no DevTools port within 20 s and the HTML layer failed (3 of 3
  starts under full load; at normal priority they take under 1.1 s). Edge starts at normal priority again, a failed
  start is tried once more with a fresh profile, and a second failure says what to do. Edge profiles left in
  `%TEMP%` by an app that is gone (`se-edge-<pid>-<n>`) are cleaned up at start-up.
- **The fast renderer draws far more on its own**: measured against the installed helper, `backdrop-filter`,
  `-webkit-text-stroke`, `@property`, `text-shadow`, ellipsis, `text-wrap: balance` and justified text all draw as
  in Edge, so Auto no longer sends them to Edge or warns about them. Edge stays for colour filters (the helper
  crashes on them), blend modes, `position: sticky`, 3D transforms, CSS counters and vertical writing.
- **Inline SVG coloured by the page's CSS** (the Sale Badge's red, star fills, `fill: var(--accent)`): the rules
  are written onto the SVG's parts before the fast renderer draws them, instead of coming out black. Only an SVG
  animated by CSS, or styled with a selector that cannot be resolved, goes to Edge.
- **A fast-renderer failure is no longer forever**: a crash keeps that layer on Edge for the running helper only (a
  restart or a helper update tries again), a hang is tried once more first.
- **The HTML preview always shows the template you picked**: picking Rating Stars right after Sale Badge could keep
  showing an old picture (the last frame stood in for the new page, and a switch between Edge and the fast
  renderer left it there).
- **Gallery HTML cards appear at once**: their pictures were decoded by one ffmpeg.exe each, one at a time
  (a blank project: 6-9 s for the HTML tab, over 2 minutes while a build kept the PC busy); now in-process on a thread that is not starved (1.6 s with every core busy). Installed HTML
  layers and icons without a picture of their own get one drawn by the fast renderer and cached.
- The Inspector says which renderer drew a layer and why, and offers the one-click fast-renderer install more
  plainly.
### Settings
- **Cache folder** (Settings ▸ General): see where renders, pre-renders, proxies, peaks and models are kept, their
  size and the drive's free space; **Browse…** to another folder and **Move existing cache** (a low-priority
  background job that skips files still being written and is safe to stop or resume) or **Start fresh** (optionally
  deleting the old folder). Takes effect at once; **Open folder**, **Reset to default**. `settings.get/set
  cache_folder`.
- **Web browser** (Settings ▸ General): links and the page-snapshot steps use your Windows default browser, or the
  one you pick (any installed browser, or Other…). `settings.get/set web_browser`.

### Web pages
- **Works with any browser**: Web Page Snapshot ▸ From my browser shows the steps for your browser (its own
  extensions page: `chrome://extensions`, `edge://extensions`, …) and watches its download folder; Firefox, which
  cannot save a page as one file, gets the Ctrl+S ▸ Web Page, complete route instead. No browser or site is
  named in examples any more (example.com, neutral sample captions and hashtags).
### Audio visualizers
- **Add with Audio Visualizer**: right-click a song in the Media Library to put it on the timeline at the
  playhead together with a visualizer layer over the same span, in the style you used last (Neon Orb at first).
  The two are linked like a video and its own audio: moving, trimming or deleting one takes the other along, and
  the export renders the visualizer with the song. **Add Audio Visualizer** on a timeline clip links too.
  `audio.visualizer {asset_id}` does the same for the Assistant.
- **Band mixer**: tune which frequencies drive a visualizer and how strongly - six bands from Sub to Air with
  their own intensity, sensitivity, attack / release, and which bands the beat follows (the kick or the snare),
  with presets for EDM, Hip-hop, Rock, Vocal, Lo-fi and Classical. A spectrum of the song shows what it has.
  Changes show at once and are saved with the project; custom shaders get the mixed values.
- **3D camera for Voxel Blocks** (and custom shaders marked `// @camera`): with the visualizer selected, drag in
  the Preview to orbit, Shift+drag to pan, scroll to zoom; the Inspector's Camera section sets yaw, pitch,
  distance, field of view, roll and target, each keyframeable for fly-throughs, with auto-orbit and Reset camera.
  Preview and export always use the same camera, and moving it costs no playback frames.
- **Fixed: the Source monitor's visualizer lagging the music for seconds** (on a song's first play, and after a
  loud part or a seek). It waited for the waveform to finish computing and followed the speakers through a
  slow auto-gain; now every visualizer - Source monitor, timeline and export - shows the song's own analysis
  at the exact moment playing, so it is in step from the first frame, after seeks and after buffering.
- **Fixed: Spectrum Bars and Voxel Blocks upside down in the Source monitor.** The monitor now draws styles the
  same way up as the timeline and the export.
- **Social guides follow playback**: the guide's play button shows pause while the editor plays, and its progress
  bar and time ("0:42 / 3:51", hours on hour-long videos) track the playhead as you play, scrub or seek. The total
  is the whole timeline. `render.frame`, `render.contact_sheet` and `guides.preview` draw the bar and time of the
  exact frame they render and take `playing` to choose the button.
- **Burn in social guide** and **Burn in captions** in Export ▸ Advanced: draw the chosen guide over every
  exported frame (playing, its bar and time following the exported video or In/Out range, without the editor's
  labels), and choose whether the subtitles are drawn in (until changed it follows the preview, as before).
  MCP: `export.video` takes `guide` / `captions`; `settings.set` takes `export_guide` / `export_captions`.
- **Premiere Pro, DaVinci Resolve and CapCut themes**, each in a dark and a light variant (greys, accents, clip colours, playhead and corner radii), in Settings > Appearance > Theme. **Import theme** now also reads Premiere Pro preferences (UI brightness and highlight colour) and CapCut preferences (dark/light); Resolve has no theme file and says so.
### Editing skills and styles (online library)
- **Technique skills**: `/jl-cuts` (split edits by rolling only the sound cut), `/pacing` (shot-length targets per
  style), `/loudness` (web -14, podcast -16, broadcast -23 LUFS), `/color-grade` (normalise, balance, match, then a
  look), `/recap`. `/captions`, `/cut-silences`, `/shorts`, `/b-roll`, `/sync-to-beat`, `/clean-audio`,
  `/highlight-reel`, `/review` and `/color-match` now use published numbers (subtitle line length and reading speed,
  noise-floor thresholds and tone presets, beat-versus-bar cutting, duck depth); `/color-match` had the grade
  before/after switch backwards.
- **Style skills** with a visible style card: `/style-vlog`, `/style-documentary`, `/style-music-video`,
  `/style-podcast-clips`, `/style-trailer`, `/style-shorts`.
- **`/convert-from-editor`** maps a Premiere Pro, DaVinci Resolve, Avid or After Effects workflow onto Simple
  Editor's tools (with alternatives and a not-supported list) and can save it as a skill; converted with it:
  `/reframe`, `/audio-to-keyframes`, `/deliver`, `/apply-cdl`. Guide page: Editing skills and styles.
- Skill headers can carry `tags` (searchable in the Gallery) and `credit` (kept in the library catalogue); a test fails the build when a library skill
  names a tool or argument that does not exist.
### Fixed (found by live-testing the assistant skills)
- **Duplicate** put the audio copy of a linked clip on a video track when a new video track had to be made for the
  picture copy.
- **Filler removal** no longer cuts into the words on either side of a filler (the padding stops at its neighbours).
- **Scene-cut detection** no longer reports a cut at the very first frame (splitting there made a one-frame clip).
- **Transcription status** says "Transcribing…" once the audio is extracted, instead of "Extracting audio…" for the
  whole run.
- Assistant tools: markers an agent creates inside its undo group are signed with its name, not shown as yours;
  `edit.begin` takes `idle_s` (the group closed after 30 s without calls while an agent looked at renders);
  `audio.analyze` also reports the clip's gain and the levels after it; `library.apply` takes `intensity` for
  LUTs; `render.contact_sheet` keeps tiles on the last frame instead of a black one past the end; transcript files
  are kept per editor process, so two open editors no longer overwrite each other's; `history.restore` and
  `tracking.run` descriptions say what they really do.
## Unreleased

### Animation templates
- **Built-in animation templates** in Gallery ▸ Templates, made from the Animator's blocks and transitions and sized to your project: Photo Slideshow, Beat Montage, Title Intro, Product Showcase, Before and After, Quote Card, Lower Third Opener, Vertical Short Opener, End Card, Photo Grid.
- **Slots**: a template's footage clips are slots. Clicking a template opens **Use Template**, where each slot takes media from the Media Library (the selection fills them in order, or drag media onto a row) or a file; Place puts the finished sequence on the timeline as one undo step. A slot either trims footage to its length or stretches the template to the footage.
- **Template Builder** (Clip ▸ Save as Template…, on the selection or the whole timeline): name, card picture time, which clips are slots (named, reordered), and per clip whether its footage is included or replaced by a placeholder.
- Templates now keep their track order, transitions and animation blocks with their Animator controls. Templates saved before still load and place as they did.
- **Share a template** as one `.setemplate` file, with or without its footage (right-click a template card ▸ Export…); import it from the Gallery's Import… or by dropping the file on the window. Footage that is not in the file becomes empty slots; no folder path of your PC is written into it.
- MCP: `templates.slots`, `templates.use`, `templates.build`, `templates.export`, `templates.import`; `templates.list` / `templates.apply` know the built-in ones.
- Fixed: with several clips selected and the playhead on a frame where the first one's scale is 0 (the start of a Pop In), the Inspector's Scale box counted showing the value as an edit and shifted the scale of every selected clip.

### Animate page and the Animator
- New **Animate** page (after Edit, Alt+4): Gallery, Preview, Inspector, a thin Timeline and the new **Animator** panel. The page bar gets it once on existing installs; Color and Audio move to Alt+5 / Alt+6 and Export keeps Ctrl+E.
- **Animator** panel: Blocks (one lane per selected clip; drag, stretch, snap with a guide line, open a block to drag its keys; blocks that overlap stack in rows), Curves (the block's keys as curves with bezier handles, box and Shift select, ease presets, value or velocity), Flow and Speed. Narrow, it becomes a list of the clip's blocks with their controls.
- **Animation blocks have controls**: every Gallery animation preset is now a block with Side, Strength, Length and Ease in the Inspector; changing one rewrites its keys. Projects from before keep their keys untouched.
- **Simulated blocks**: Camera Shake (jolts per second, fade out, tilt, new pattern, hide edges), Spring In, Bounce Drop, Throw Out and Wobble.
- **Flow**: wherever one move hands over to the next (on one clip or across a cut) a ⚡ or ✓ badge shows whether the speeds match; Smooth eases the next move, the earlier one or both so they meet. The ease editor's handles snap to the matching speed.
- **Speed ramps** are real time remapping now (the source plays the rate summed over time, so a ramp no longer jumps); new Speed Ramp In / Out, Slow-mo Hit and Speed Up blocks and "Ramp through the cut".
- **Sound blocks**: Duck, Pan Sweep, Pan Swing and Tremolo; Pan can be animated by blocks.
- **Stacks**: Clip ▸ Nest ▸ Stack Clip with Overlays groups a clip and its overlays into one nested clip on the clip's own track, so a transition moves them together.
- **Links**: Link to layer (a property follows another clip's, x multiplier + offset), Turn along path, and "Draw a motion path" (the next pencil stroke in the Viewer becomes a path the selected clips follow).
- **Onion skin** in the Preview while the Animator is open: the selected layer's outline a few frames before and after, and where its block starts.
- Presets: save a block to the Gallery, import presets, open the preset folder to share the files.
- MCP: `animate.stack`, `animate.link`, `animate.follow_path`, `keyblocks.set`, `flow.seams`, `flow.match`.
- **Hand keys to blocks**: keys you made by hand show as a dashed "Hand keys" row in the Animator; click it (or right-click the clip ▸ Turn hand keys into blocks, or select keys in Curves ▸ Make a block) and they become a Custom block with strength, ease and side controls, the motion unchanged. MCP: `keyblocks.from_keys`.
- The Animator gives each clip a lane per kind of property its blocks animate: Motion, Opacity, Speed, Sound.
- Motion and speed-ramp presets drop as blocks; timeline clips no longer draw keyframes (a thin colour strip marks an animated clip).
- Fixed: `timeline.move` ignored `dtrack`; `timeline.trim` with both `start` and `end` limited the end by the media left before the start moved; `clip.keyframe` / `clip.set` property names now accept the short names (`x`, `X`, `position_y`, `speed`, …).
- **A block is a move from an origin to a target**: From / To fields in the Inspector set where it starts and ends; the ease is just how it travels between them. Open a block (▸) and it shows that move as one string: pull the two middle points to shape the ease, an end point sideways to stretch it. Keyframes only show in Curves.
- **Split a block** at the playhead (Ctrl+B with a block picked, or right-click ▸ Split at playhead): the first part ends at the value there, the second starts from it and keeps the old target. **Flow together**: Shift / Ctrl-click several blocks, then each one starts where the one before ends and their speeds are matched.
- Animator: Selected clips / Whole timeline switch for the lanes; with Snapping on a dragged block also snaps to every clip's edges (off: no snapping); a **Record** switch (off: a value change never adds a key, keys are placed by hand with ◆ or a double-click on the property name); adding a block no longer starts playback.
- Fixed: Delete / Backspace with a block or keys picked in the Animator also deleted the timeline clip; an ease picked for a block only changed its first segment (and nothing on the last key of a move), and the right-click Ease was lost on the next control change; changing a value away from a key littered an animated property with keys.
- **Link into one block**: with several blocks of one clip picked, they become a single block with all their properties (the motion stays). The Curves channel list can be resized (drag the line between it and the graph).
- Fixed: the Sound and Speed group rows of the Curves channel list shared widget ids (red "ID clash" boxes on a sound clip).
- MCP: `keyblocks.split`, `keyblocks.link`, `flow.together`.
- **One timeline for Blocks, Curves, Flow and Speed**: all four views are the lanes timeline now, with the same ruler and playhead, Selected clips / Whole timeline, the zoom bar, Ctrl+wheel zoom, Shift+wheel pan and a new scrollbar under the lanes. The view only changes what each clip's row shows: Curves puts the clip's channel list in the label column and its keys in the lane, Flow its speed graph with the hand-off buttons, Speed its speed ramp (the ramps are in the row's "Ramps" menu). Several selected clips are stacked rows under one ruler; drag a row's bottom edge (or Alt+wheel) for taller graphs.
- **Loop**: a block's Inspector has Loop (how many times, and ping-pong for there and back); right-click a block ▸ Loop does the same. The bar grows to that many times its length and shows a tick per cycle. In Curves, select keys ▸ Loop keys (or right-click ▸ Loop) repeats them after the last one.
- **Physics**: new Gravity Fall (falls from where it is and bounces on the bottom of the frame, optionally thrown to a side first) and Throw & Bounce (thrown across the screen, bouncing off the frame's edges) blocks; a simulated block's Inspector card has a Physics section with its sliders and a preview of the motion. **Drop with collisions** (with two or more pictures selected, or ⋯ ▸ Drop with collisions ▸ Normal / Bouncy / Heavy / Floaty) lets the selected clips fall, land and push each other apart; each gets a "Collide" block of ordinary keys.
- **Rigid body**: Gravity Fall, Throw & Bounce and every clip of a collision drop are a simple rigid body with the same controls in the Inspector: Mass, Gravity and the way it pulls, Elasticity, Friction, Air drag, Velocity (speed and direction) and Spin (baked as rotation). Mass matters: a heavier body is slowed less by air drag and pushed less in a collision. Changing a Collide block's body runs that drop again for every clip in it.
- **A colour per kind of animation**: In teal, Out orange, Physics purple, Collide pink, Emphasis yellow, Camera blue, Text sky, Sound green, Speed red, Custom slate. The block's bar, its Inspector pill, its Gallery card and the timeline's strip all use it, and the "+ Animation" menu shows each kind's dot.
- Animator: zoomed all the way out, "Selected clips" is exactly the selected clips (first start to last end, no air) and "Whole timeline" exactly the timeline; the range switch sits on the right, next to the zoom.
- Fixed: dragging a row's handle in the Curves channel list did nothing. It reorders the list now (a property inside its group, a group among the groups; Alt+Up / Down too) and the curves are drawn in that order.
- Curves: hold **Shift** while dragging keys to lock the drag to one axis (sideways = time only, up and down = value only). Row titles read "Properties - <clip>" (or "<block> - <clip>").
- Fixed: keys in Curves could not be dragged (or stay selected) unless a block was picked in the Animator.
- MCP: `keyblocks.loop`, `keyblocks.loop_keys`, `animate.collide`; `keyblocks.set` physics takes `throw`.
- **Inspector: a clip's properties are one aligned list**, the Curves channel list's rows: Transform, Opacity, Sound and Speed fold, and every row has the same columns - colour bar and name, a number box of one width (drag or type), previous key, the key button, next key, reset, and the chain toggle (Position X / Y keyed together, Scale X / Y). A longer number no longer pushes the controls. Double-click a name for a key at the playhead. Opacity, Volume and Pan are number boxes like the rest (0-100 %, dB, -1..1).
- **Path tool** (the pencil in the Animator's toolbar; D while the Animator is in use): draw in the Viewer and each selected clip gets a Path block from the playhead that travels your stroke. The block keeps the stroke: in the Inspector, Timing is Shape (one speed along the path over the block, shaped by Ease) or As drawn (your stroke's own timing, scaled to the block), Smoothing goes from the raw points to a rounded curve with the ends fixed, and Turn along path faces the clip the way it travels. Move, stretch, loop or split it like any block. "Draw a motion path…" in the ⋯ menu now picks this tool (it used to save a path the clips were linked to).
- **Timeline lane: transitions over the clips.** A bar in the new lane at the top of the Animator puts its transition on every cut under it, closes the gaps between the footage clips under it first (the lowest video track; overlays keep their places), and keeps doing so wherever you move or stretch it and whatever changes under it; delete it and its transitions go. Add one from + Animation ▸ Transitions ▸ Timeline (over the range shown) or a right-click in the lane: the built-in kinds and the installed library transitions. Its Inspector card sets the transition, each one's duration, ease and direction. The lane shows without a selection. Overlays are not grouped into stacks yet: each video track gets its own transitions.
- **Show Animations** on a clip's right-click on the timeline opens the Animator on the selected clips.
- MCP: `animate.path`, `animate.span_add`, `animate.span_set`, `animate.span_remove`, `animate.spans`.
- **Convert a whole timeline to blocks and spans in a few tool calls** (the Assistant can now do "convert my keyframes to animation blocks and simplify the transitions" on its own): `animate.audit` reads, in one small paged answer, which clips carry hand keys, which keys blocks own, and how the cuts are set up (with suggested spans); `keyblocks.convert_all {all: true}` turns every run of hand keys into named blocks with controls where one matches exactly (Fade In / Out, Slide, Pop, Zoom Punch, Ken Burns, Sound Swell, ...) or a Custom block otherwise, checks every property before and after so the motion never changes, and is one undo step (`dry_run` previews); `animate.spans_from_transitions` turns each run of cuts sharing a transition into one span at one length, leaving hard cuts hard (`include_hard_cuts` to fill them). `timeline.list` takes filters (`track`, `from`/`to`, `kind`, `has_keyframes`, `has_blocks`, `fields`, `limit`/`offset`). New built-in skill `/blocks-from-keys`.

## 1.3.0

### Performance: peak playback
- **Nested sequences play on the GPU**: a sequence clip is drawn by the graphics card with the same shaders as the
  timeline, instead of a full copy of the project rendered on the processor every frame.
- **Zero-copy hardware decode**: decoded video stays on the graphics card (D3D11 → OpenGL interop) at every size, so
  1440p 120 Mbit footage decodes about 2.5x faster (6.2 → 2.5 ms a frame) and 1080p about 3x (4.7 → 1.6 ms).
- **No stall at cuts**: the decoder for the next clip is opened and seeked ahead of the cut (a cut deep into a long
  recording went from a 2 s freeze to a 27 ms worst gap), and the read-ahead grows when frames are expensive.
- **Opening big projects no longer freezes the editor**: the HTML-layer scan re-processed every saved web page every
  2 s on the interface thread (minutes of freeze on a big project); now cached. Paused interface frame p95 6.95 ms.
- **Background work never competes with playback**: proxies, HTML renders, thumbnails and transcription run at low
  priority and pause while you play; proxies are built nearest-the-playhead first and only for footage that needs
  one (over 1080p, over 50 Mbit, intra codecs like PNG / ProRes), with the hardware encoder; per-clip Proxies
  auto / on / off in the Library menu.
- **One disk-cache budget** (Settings ▸ General, default 30 GB) with least-recently-used cleanup across HTML
  renders, pre-renders, proxies and waveforms, a low-disk warning, and pre-renders that only redo the sequence you
  edited. `caches.prune`.
- **HTML renders are ~50x smaller**: animated HTML layers render to H.264 (GPU encoder when present; transparent
  layers as stacked colour-over-alpha) instead of lossless PNG movies, at the size they are shown (never above the
  project size), superseded renders are deleted after an edit, and HTML renders get their own 1 GB cap. Old PNG
  renders are cleaned up in the background; each HTML layer re-renders once in the new format.
- **Performance window** (Window ▸ Performance…): live FPS in a donut chart of where each frame's time goes,
  live tiles (dropped frames, worst gap, decode, cache, background jobs) coloured against the budgets, and a
  Project tab ranking the heaviest sources and clips with Jump to and one-click fixes (Make proxy, Pre-render,
  Free cache, Pause jobs). For AI agents: `perf.bench` (play a range in the live app and get dropped frames,
  fps, worst gap, UI p95 and the cost breakdown against the budgets), `perf.report`, `perf.window`, `playback.trace`
  (Chrome trace).
- **Performance benchmark and gate**: `scripts/perf-bench.ps1` plays a copy of a project (or a synthetic stress
  project) in a scratch profile and records `perf_log.csv`; `/se-verify` fails on dropped frames or regressions.

### AI enhancer (optional add-on)
- **AI Upscale in place**: right-click a video or picture clip ▸ AI Upscale ▸ ×2 / ×4, Game / Animation
  (`realesr-animevideov3`) or Real-World (`realesrgan-x4plus`). Real-ESRGAN runs on the graphics card over
  just the part of the source the clip shows (plus 1 s handles), at low priority, as a job with progress and
  cancel; then the clip plays the upscaled file with its effects, transform, keyframes, transitions and timing
  kept (one undo step). The new file sits beside the original (untouched) and in the "AI enhanced" bin. A clip
  zoomed past 1.2× names its zoom in the menu. The upscaler is an **optional add-on** (about 45 MB, downloaded
  only when you click Install in Settings ▸ General or accept the offer), so the app itself does not grow.
  `enhance.status`, `enhance.install`, `enhance.upscale` MCP tools.

### Audio visualizers
- **Add Audio Visualizer** (right-click an audio clip ▸ Audio, or `audio.visualizer`): a full-frame layer over
  the song that moves with it. Pick the style in the Inspector's Visualizer section: **Neon Orb**, **Spectrum
  Bars** (the Source monitor's original bar field) or **Voxel Blocks** (a pixel-art block island whose columns
  follow the spectrum, with a block dropping in and stacking on every beat, an orbiting camera, four palettes
  and an optional transparent background). The music is analysed once into keyframes, so playback never pays
  for it and the export matches the preview.
- **Your own visualizers**: any GLSL effect in your library that declares `level`, `band1`..`band6`, `beat`,
  `beats` or `bpm` shows up as a style. Shaders may now have up to 14 controls.
- **Now playing**: sound with no picture and no visualizer shows its cover art and title / artist (or the
  music-note icon) in the Source and Program monitors. The Source monitor's visualizer style is pickable from
  its right-click menu.

### Chat, forum and video-comment threads
- **Lists are tables now** (a rule for every list in the app): UI list/row layout: any vertical list of rows is laid out as a table with fixed, aligned columns [drag handle][arrow][label, fills and truncates][fixed value columns][fixed right-aligned controls]; controls never float after variable-length text; form fields use two aligned columns (labels at one x, inputs at one x); reorder = drag handle + up/down + Alt+Up/Down, one undo step; use the shared helper (`ui::list_rows`). The thread editor's messages, comments and cast follow it; drag a row's grip to reorder (replies move with their comment, and a horizontal move picks the nesting), Esc cancels.
- **Messages only, DMs, typing and video comments**: a chat is now just the people messaging by default;
  server rail, channel / conversation list, member list, top bar, header, message box, typing line, date
  dividers, NEW line, scrollbar and a transparent background are each a switch, with Server channel / DM /
  Group DM layouts. New **typing animation**: your messages type into the message box key by key (speed,
  typos with backspace, caret), then send, while the others "are typing"; all CSS timing, frame-exact. New
  **Video Comments** thread (comment box, comments, likes, hearts, pins, replies) whose comment can be typed,
  the Comment button pressed (the fake mouse can aim at it) and posted. Six Gallery cards.
- **Fake chat and forum threads** (Add ▸ Chat Thread / Forum Thread, the online library, or `thread.create`):
  build a chat window or a forum post with nested comments from a list instead of HTML. Inspector ▸ Thread has
  + Add message / comment / reply, move, duplicate, delete and fold per item; a cast of people with avatars
  from your project, name colours and badges (change one and all their messages follow); reactions, embeds,
  replies, system lines, typing indicator, six looks, themes, fonts and times; a script box that turns plain
  lines into messages, and JSON import / export. Items arrive one after another with exact CSS timing (pop,
  slide, fade, typing dots, vote counts counting up, pulse), the newest always in view. `thread.*` MCP tools.

### HTML layers
- **A fast, small renderer for HTML layers** (optional, 11 MB): the app can draw HTML layers with its
  own helper (`se-html`: Blitz, no browser, no JavaScript) instead of Microsoft Edge: ~25 MB of memory
  instead of ~700 MB, a picture in milliseconds, no Edge process to start. Inspector > HTML > Renderer:
  **Auto** (default) uses it when it is installed and the layer only uses CSS it can draw, else Edge,
  and says why ("uses a backdrop filter: rendering with Edge"); Fast and Edge force one. Nothing
  downloads until you press **Install** on the card the Inspector shows (the helper lives in
  `%APPDATA%\SimpleEditor\bin`, size and SHA-256 checked); Edge keeps working meanwhile and is the
  fallback if the helper fails on a layer. Export, the timeline render, live preview, `html.check`,
  hover and the fake mouse all go through it; web page snapshots are still taken by Edge (the page runs
  scripts once) and drawn by the helper afterwards. New tools `html.renderer`, `html.helper`; the
  library templates were rewritten where they needed CSS the helper cannot draw (same look).
  `simple-editor.exe` itself does not grow by a byte of Blitz: the helper is its own program.
- **Long HTML layers render many times faster**, always at the project's frame rate: several copies of
  Edge share the work (up to 4), and frames where nothing moves are reused. A 60 s layer shows a still
  at once, a draft of the whole layer within seconds, then the 10 s around the playhead at full rate
  (`html_render_window_secs`), then the rest. Edge starts in the background when a project with HTML
  layers opens, and its processes now always close with the app.
- **Live preview**: the Inspector's new Preview and the HTML editor show the page itself running, so a
  field, parameter or code change shows at once while the timeline's frame-exact render catches up. Each
  has its own player: a time bar, play / pause, frame steps, start / end, loop, follow playhead, timecode
  (Space, Left / Right, Home / End); `html.preview` sets it over MCP.
- **More fields for HTML layers**: font (installed and the app's own fonts, previewed as you hover),
  select, toggle, range / radius / size sliders, shadow and outline. The library templates use them for
  font, corner radius, shadow and outline, with their look unchanged.
- **Your project's files by name**: `<img src="logo.png">`, `<video src="clip.mp4">`, CSS
  `url("bg.jpg")` find the project's file of that name; missing names are listed in the Inspector.
  Videos inside a layer are frame-exact on the timeline and in exports.
### Playback
- **Buffer before playing**: when the frames just ahead of the playhead are not decoded yet (the first Play
  after opening a project, after a seek, over 4K or stacked layers, effects not compiled yet), Play now waits
  up to 3 seconds for the next second of picture and sound, shows "Buffering" over the Monitor, and starts by
  itself, instead of stuttering with the audio running ahead. When it is ready Play starts at once as before.
  Space cancels it. Settings > Performance > Buffer before playing turns it off.
### Library
- **Import a folder as a bin.** Drop a folder from Explorer on the window, click the folder button next to
  **+ Import**, or use **File ▸ Import ▸ Import Folder as Bin…** (also in the command palette, the empty-spot
  menu, and **Browse**'s right-click ▸ **Import folder as bin**). The folder becomes a bin named after it with
  every video, audio and image file inside in natural name order; sub-folders become nested bins (8 levels).
  Hidden/system files and unsupported types are skipped and counted in the toast ("Imported 84 files into
  'SFX' (3 skipped)"), files already in the Library are reused, it runs in the background, asks above 500
  files and is one Undo step. A numbered PNG run in a folder still bakes into one clip, filed in its bin.
- MCP: new `media.import_folder` (`path`, `recursive`, `bin_name`, `extensions`); `media.import` also accepts
  folders in `paths`.
### My Gallery
- **Save to Gallery…** (right-click a clip, an effect row in the Inspector, a transition or an empty spot in
  the Gallery; also Clip menu and the `gallery.save` tool): keep your own effect stacks / single effects,
  transitions (with duration and shader controls), looks, caption styles, speed ramps, keyframe animations,
  titles and templates, HTML layers, shader effects, LUTs and sounds. They show in the matching Gallery tab
  marked **M**, with a **Mine** filter; right-click one for Rename, Edit tags, Duplicate, Replace with
  current, Open file location and Delete (to a `.trash` folder). The old save paths (`stills.save`,
  `keyblocks.save`, `templates.save`, Save Template) now write the same files.
- **Plain files you can share**: `%APPDATA%\SimpleEditor\gallery\<kind>\<name>.json` (a sound is the audio
  file with a small `.json` beside it) and an `index.json` that is rebuilt from the files when missing. Items
  that used to live in `settings.json` move there once. **Folder** and **Import…** buttons per tab; drop
  files, folders or a `.zip` onto the Gallery to import; a taken name is numbered, never overwritten.
- **Sounds tab**: your sound effects as cards with waveform and length, preview on hover, click to add at the
  playhead, drag onto an audio track (or below the last track for a new one). Nothing is decoded at start-up.
- New tools: `gallery.save`, `gallery.import`, `gallery.manage`, `gallery.reveal`; `gallery.list {mine}`;
  `gallery.apply {tab: "Sounds", at, track}`.

### Gallery
- **One grid, one header, everywhere.** Every Gallery tab (Animate, Looks, Luts, Captions, Speed, Effects,
  Transitions, Templates, Titles, Sounds, HTML, Icons, Skills) is a wrapping grid that fills the panel and
  reflows when you resize it, with the tab strip and a search header pinned above it: one search box (names,
  tags, descriptions; <kbd>Ctrl+F</kbd> over the pane focuses it, <kbd>Esc</kbd> clears it), **All / Built-in /
  Mine / Installed / Online** chips, a category menu (effect categories, transition kinds, tags), a sort order
  (Default, Name, Recent, Most used), the result count and an **S / M / L** card size.
- **The online library is not a separate screen any more**, and nothing is fetched until you ask. The
  "use online library or a folder" choice is gone: each tab shows the built-in cards, yours, what is installed
  and, after you click **Browse online library** once (remembered), the online items - the small index only.
  Clicking an online card downloads just that item (progress on the card) and uses it. **Use a folder** and
  **Turn the online library off** moved to the tab's **⋯** menu. The Library panel no longer repeats the
  Effects / Adjustment-layer lists the Gallery has.
- New tool `gallery.connect`; `gallery.list` takes `source`, `query`, `category`, `sort` and `detail` (and
  lists library items); `library.list` shows only installed items until the library is connected.
- **Effects and Transitions are Gallery tabs now**; the separate Effects and Transitions panes are gone.
  Each tab lists the built-in cards and the online library's (installed or not, cloud mark) in one grid under
  one search. <kbd>Ctrl+3</kbd> / <kbd>Ctrl+4</kbd> open the Gallery on that tab. A saved layout, page or
  profile that still has the old panes loads with one Gallery tab where they were; `layout.surface`,
  `layout.pin` and `ui.screenshot` still accept the names "Effects" and "Transitions" (they mean the Gallery).
  New tool `gallery.tab`; `gallery.apply` takes tab `Effects` and, for Transitions, `where`
  (`start | end | connection | all`).
- **Drag any Gallery card onto the timeline**: effects, looks, LUTs, caption styles, speed ramps and keyframe
  blocks onto a clip (the whole selection when you drop on one of several selected clips); transitions onto a
  cut or a clip edge (snaps to the nearest edge); templates, titles, HTML layers and icons onto an empty video
  track or the lane below the last one. The target lights up while you drag, <kbd>Esc</kbd> cancels, a
  library item that is not installed yet downloads first, and a click still just applies the card.
- **Right-click a transition card**: Apply to Start, Apply to End, Apply to Connection (when the clip touches
  another one) and Apply to All (with two or more clips selected). The length is never more than half of the
  shorter clip it touches.
- **Fixed: the Animate tab ran off the right edge in one row**, so most keyframe blocks could not be reached.
  Every Gallery card now wraps to the pane's width and the pane scrolls down to the last row.
- **Browser window layer** (Clip ▸ Add ▸ Add Browser Window, or Gallery ▸ Titles ▸ Browser Window):
  everything on the tracks below shows inside a drawn browser window with tabs, an address bar, a load
  bar, a shadow and a background. Looks: Light, Dark, Minimal, Mobile. Scroll and zoom the page with
  keyframes; the address can type itself out; **Next page here** cuts the page and runs the load bar.
- **Fake mouse** on the browser layer: arrow, hand and text pointers, click ripples, drags, wheel marks
  and a keystroke overlay. **Record mouse** plays the timeline and turns your real mouse over the viewer
  into a few editable keyframes and click marks (re-record any part); or click waypoints on the viewer.
- **Import Web Page…** (Clip ▸ Add): a real page by address or from a saved `.mhtml` / `.html` file becomes a
  real HTML layer under a browser layer (effects, masks and fields work on it), stored on your computer
  with its scripts removed, with a map of its links and buttons. **Make editable** turns its headings
  and labels into fields. **Sign in…** opens a browser profile kept only for Simple Editor so you can import a
  page as you see it; the editor never sees your password.
- **Scroll and zoom without blur**: scrolling and zooming a browser layer enlarge the page itself, and
  **Sharpen zooms** redraws an imported page at exactly the zoom you rest on (crisp at 800 %).
  **Zoom in ×2** zooms to where the mouse is.
- **Go to it** and **Click and go**: the mouse finds a link or button by its text, eases onto it with a
  slight overshoot, and one step clicks it and changes to the next page with a fade, a load bar and the
  new address and tab title.
- **Window zoom** on a browser layer: the whole browser and the mouse cursor zoom together, redrawn at the
  zoomed size (`browser.animate {preset: "zoom_to", whole: true}`).
- **Display ▸ Page only** on a browser layer: just the page, edge to edge, no window around it.
- Fixed: a polygon mask on a clip was drawn in the wrong place in the preview (it was right in a nested
  sequence and wrong everywhere else).
- 7 new tools for scripts and the assistant, most with an `action` argument: `browser.add`, `browser.set` (read or
  change), `browser.animate`, `browser.page` (fit, next, click_go, elements, sharpen, make_editable),
  `browser.import` (import, jobs, sign_in), `cursor.set` (move_to, path, event, press, tidy, samples) and
  `cursor.record` (start, stop).
### Social guides, remade from the real apps
- **Measured, not guessed.** Every platform's guide was re-made from its real logged-out web pages
  (buttons, text sizes, bars, the comment sheets) and the platforms' own safe-zone documents, and stored
  as one small data file with a "verified on" date and where it came from. New: YouTube long form (title
  bar, controls, captions, watermark, end screens), Instagram post (4:5 and the 3:4 / 1:1 grid crops),
  Facebook Reels, Snapchat Spotlight and X video, besides TikTok, Reels and Shorts.
- **Three modes** (right-click the Preview > Social Guides): **Safe zone** (danger, caution and safe
  areas with percentages, plus the phone's status bar and home indicator), **UI preview** (the app's
  buttons as plain silhouettes with your profile filled in), and **Device preview**.
- **Device preview**: your video inside a drawn iPhone, Android, small phone, tablet or desktop browser
  with the app's UI on top. Click **Comments** and the sheet slides up while the video shrinks into the
  part above it (where the app does that); click the caption, avatar, sound, Share or the three dots for the
  other sheets; the progress bar follows the playhead. View only: it never changes the project or the export.
- **Desktop player views**: with the Desktop browser device, YouTube long form shows its default, theatre and
  fullscreen layouts (measured on the real site); click the control bar's buttons, press T / F (Esc leaves
  fullscreen) or use Social Guides ▸ Player view, and the player glides between them.
- The device is clipped to its rounded screen and body: no video, silhouette or sheet corner is drawn outside it.
- **Worst case of all platforms**: one safe area for every vertical platform at once.
- **Profiles** (Settings > Social): handle, name, picture, verified badge, followers, caption, hashtags,
  sound, counts and sample comments, several per network. **Fetch from my public profile** reads the
  public page's name, picture and follower text (no login, no cookies, only when you click, and it shows
  exactly what it found before it applies anything).
- **Updatable without a release**: drop a newer `guides.json` in `%APPDATA%\SimpleEditor\library\guides`, or
  Settings > Social > Check the library for updated guides.
- MCP: `guides.list`, `guides.set`, `guides.preview` (a picture of the device with a sheet open),
  `profile.get`, `profile.set`, `profile.fetch`; `render.frame` and `render.contact_sheet` take `guide`.
- Old settings load unchanged (the five old guide names map to their new platforms).

## 1.2.0

The biggest release yet: an AI **Assistant** that edits your project while you watch, HTML + CSS
layers, shader effects and transitions, an online library, web page snapshots, faster and smaller-file
exports, and an easy way to report bugs.

### Creator Kit
- **HTML + CSS layers and scenes** (Add ▸ HTML Layer / HTML Scene): titles, lower thirds, buttons and
  cards written in HTML and CSS (no JavaScript), rendered by the Microsoft Edge that ships with Windows,
  with transparency, and exported with the video. A template's texts and colours show as **Fields** in
  the Inspector; double-click the clip for the HTML editor (live preview, scrub bar, Ask the assistant).
- **Shader transitions** in the gl-transitions format and **shader effects** with named controls
  (`// @param glow 0.5 0 2`), added without rebuilding the app. The Shader Editor can save to the library.
- **Online library** in the Gallery (Transitions, Effects, LUTs, HTML, Icons, Skills): 40 transitions,
  40 effects, 30 LUTs, 36 HTML layers and scenes, 40 icons and 21 skills. Items with a cloud mark download
  on first click; what a project uses is stored in the project (LUTs are copied into a folder beside it).
- **Assistant skills**: type `/` in the chat or the Tasks box (`/captions bold yellow`, `/lower-third`,
  `/shorts`, `/create-skill …`); a Skills tab to run, edit, duplicate and write your own. Skills are
  Markdown files in `%APPDATA%\SimpleEditor\skills`. Quick prompts you wrote yourself become skills.
- **HTML clips look and edit like HTML**: a `</>` mark and no filmstrip on the timeline; the Inspector shows
  the code with syntax colours and completion; the editor is a real, maximisable window (F11). Link a clip to
  a `.html` file (edit either side), open it in your code editor, or import / drop `.html` and `.mhtml`
  files; linked CSS and images are followed and re-render when they change.
- **Parameters** (`<meta name="se:param" content="name | number | default | min | max">`): animatable
  numbers, the CSS variable `var(--name)`, keyframed in the Inspector or with `html.set`.
- **Web Page Snapshot** (Add ▸ Web Page Snapshot…): a real page by address (the bundled Edge keeps the HTML
  and CSS, drops scripts) or, signed in, from your own browser with the small extension in
  `tools/browser-extension` (saved to Downloads, imported automatically). Scroll, zoom, animate it.
- **Fake mouse** (Inspector ▸ Mouse): keyframe a cursor's position and press; real hover and `:active`
  styles and their CSS transitions play, frame-exact. Links go nowhere.
- MCP: `html.*`, `clip.set_shader`, `shader.check`, `transition.set_shader`, `skills.*`, `library.*`.
- Fixed: the outgoing clip of a transition went black after the cut when it used its file to the last frame.

### Work with an AI agent
- New **Assistant** panel (Window ▸ Assistant): type a request and an AI agent edits the open project
  while you watch. Runs Claude Code, your own command (Codex or any MCP agent), or waits for an agent you
  connected yourself. Settings ▸ Assistant holds the agent, model, an optional API key (kept in Windows
  Credential Manager) and the quick-prompt buttons.
- Context chips show what goes with a request (selection, In/Out, playhead, a comment); click one to leave
  it out. Type `>` in the command palette to ask from anywhere.
- Every request is one undo step, with **Undo all** in the panel; the clips it changed get a short outline.
- Exports, opening projects, settings changes and scripts called by an agent wait for your **Allow**
  (switch off in Settings ▸ Assistant).
- **Add Comment** leaves a note over In/Out, the selection or the playhead for an agent to pick up and
  tick off; comments show who left them and dim once done, and range markers draw as a span.
- Voice: the mic button and **Push to Talk** dictate a request (needs whisper.cpp, like subtitles).
- MCP: `editor.context`, `edit.begin` / `edit.end` / `edit.batch` (one undo step), `render.contact_sheet`,
  `render.frame` as a real image, `assistant.*`, `voice.*`, `markers.comment` / `markers.set`,
  `transcribe.status`, and `islands` / `async` on transcription for accurate word times on long takes.

### Export
- **Speed / Balanced / Smallest file** on the Export panel.
- "auto" uses your NVIDIA, Intel or AMD hardware H.264 encoder once a one-frame test shows it really works;
  encoders that can't run are hidden.
- GPU exports read frames back while the next ones render, in the encoder's own colour format when it
  can; sound is mixed alongside the picture.
- **Audio**: one mix, separate audio tracks in the file (MP4 / MOV / MKV / M4V), or WAV stems in a folder
  beside it, split by track, bus or role.
- Surround audio is downmixed properly (the centre channel is no longer dropped).

### Hand-off
- **Export Avid EDL…** (CMX 3600, one file per video track) and **Avid markers…** in the Markers panel.
- Premiere / Resolve XML now carries audio volume and pan, A/V links, cross dissolves and markers, and
  reads its markers back on import. EDL import understands 29.97 drop-frame.

### Reporting bugs
- **Help ▸ Report a Bug…** opens the GitHub issue form with the version filled in; a short form asks what
  went wrong, the steps, and optional screenshots or `crash.log`. Ideas go through **Suggest a feature**.
  Questions go to the Discord. See "Report a bug" in the guide.

### Playback
- Decoded frames are shared instead of copied, video frames go to the GPU without a CPU colour swizzle,
  and a frame's layers decode side by side.
- The Source monitor plays through the GPU like the Preview.
- **Playback ▸ Performance HUD** shows frame, upload, decode and cache timings over the Preview.

## 1.1.0

### Preview performance
- A stack of stills over video plays in real time again: each still is decoded once (in the background at project open, capped at the largest size it is shown at) instead of re-running ffmpeg for every new size; the GPU does the scaling.
- Hardware decoding is only used above 1080p (the GPU-to-CPU copy made 720p proxies several times slower than software decoding).
- No layer blocks a frame: a source that is not ready is left out and the frame is redone when it arrives; Play and Seek pre-empt read-ahead.
- Preloading: paused read-ahead around the playhead, scrub prediction, upcoming cuts decoded early, and hovering the timeline opens the footage under the pointer.
- Memory no longer creeps over long sessions; GPU layer textures are capped at 512 MB. Invisible layers (opacity 0, off-canvas, fully covered) are skipped.

### Fixes
- Videos with an alpha channel (e.g. PNG / ProRes 4444 `.mov` wipes) keep their transparency in the preview.
- An export with overlapping MP3 clips no longer hangs at "Mixing audio…"; MP3s are resampled with a higher-quality resampler.
- Media Browser / Library folder rows are one full-width target: double-click anywhere (arrow, icon or name) expands.

### Look & docs
- GitHub Dark is the default theme, with the #335FFF accent.
- The guide's screenshots are retaken in GitHub Dark; `release-fast` build profile for quick local builds.

## 1.0.0

The first major release: a Resolve-style page workflow, a full tutorial site, and a much faster
editing loop. Everything from beta-0.3.0 below is included.

### Pages & layout
- Custom pages: add, rename, duplicate, delete and drag to reorder pages; Restore Default Pages brings the built-ins back.
- Cut page gets a planning group (Planner, Moodboard, Markers, Subtitles); Media page hides the Inspector by default.
- Closing a panel lets its neighbours expand into the space, on every page.
- Source and Preview share a tab: opening something in Source shows Source, clicking the timeline shows Preview.

### Color page (Resolve-style)
- Lift / Gamma / Gain / Offset wheels with master sliders, a clip thumbnail strip, colour curves (Luma/R/G/B) and scopes.
- One-click keyframes per wheel and "Keyframe grade" for the whole grade; the Inspector edits colour on the Color page.

### Keyframing
- 29 chainable keyframe blocks (fades, pops, slides, zoom punch, shake, spin, bounce, Ken Burns, typewriter…) in Gallery ▸ Animate; save your own; blocks show as draggable, stretchable bars on clips.
- Easing presets per key; keyframe diamonds are now perfect diamonds everywhere.

### Timeline & tools
- Premiere-style tool groups with real timeline tools: Marker (click or drag a range), Track Select Forward/Backward, Ripple, Roll, Slip, Slide, Pen, Hand, Zoom, plus masks, crop, draw and shapes. Hide or add tools from the right-click menu; a tool is always selected.
- Resolve-style deletion: Delete ripples (closes the gap), Backspace lifts (leaves it). Optional auto close gaps.
- Show Video / Show Audio tracks toggles; the Audio page shows audio tracks only.

### Library & Media Browser
- Click selects, double-click opens in Source; Shift / Ctrl multi-select and Ctrl+A; a bulk right-click menu; Add to timeline keeps your click order.
- Folder rows are full-width; double-click anywhere on a row (arrow, icon or name) expands it.

### Effects & visuals
- Shadertoy shaders paste in verbatim (mainImage, iTime, iResolution, iChannel0…) with error lines mapped to your source and example shaders.
- noa-35c3-style audio visualizer that follows a live spectrum of what's playing.
- Social guides draw only over the video, under menus and other panels.

### Look
- GitHub Dark is the default theme, with the brand accent #335FFF (no longer the Windows accent). Icon-only eyedropper.

### Other
- Import shortcuts and preferences from Premiere Pro and DaVinci Resolve (Settings ▸ Hotkeys ▸ Import from…).
- New project replaces the open one on every path, with a save prompt.
- Tutorial site at https://kashtheking.com/simple-editor/guide/ with fresh GitHub Dark screenshots.

## beta-0.3.0

### Docs site (issue #85, simplify wave 2)
A tutorial for every part of the app now lives at https://kashtheking.com/simple-editor/guide/ (Help ▸
Tutorial & Docs): getting started, the interface and pages, the Library and Source monitor, the viewer,
the timeline (with the full mouse-modifier table), the Inspector and keyframes, effects / transitions /
titles, captions, the Media / Cut / Edit / Color / Audio / Export pages, a right-click reference for every surface, the
keyboard shortcuts, advanced tools, and scripting & MCP. It's a Docusaurus site in `website/`, deployed to
GitHub Pages by `.github/workflows/docs.yml`. Every screenshot comes from the real app:
`scripts/docs-shots.ps1` builds a demo project and drives the app over MCP, one launch per shot, and
`simple-editor --dump-hotkeys` writes the shortcuts page from the F1 sheet. `layout.list` now reports
where each panel is on screen, and `SE_SCREENSHOT_WHEN=<file>` makes a `--screenshot` run wait for that
file instead of a fixed delay.

### Media and Cut pages, Inspector top right, unlocked panels
Six pages now, in Resolve's order: **Media** (Library · Source · Inspector, three full-height columns,
no Timeline) and **Cut** (Library · Source | Preview · Inspector over the Timeline) join Edit, Color,
Audio and Export on Alt+1..6 (Media, Cut, Edit, Color, Audio, Export - a rebind saved under a page
Action's id now names the page in that slot). A first run still opens on Edit. The Inspector is the
top-right tile of every page's default: Color puts it where Scopes was (Scopes moves beside Nodes |
Curves at the bottom) and Export stacks it over Jobs; pages you already arranged keep their trees.
Panels are unlocked by default - drag a tab to re-dock it - and Window ▸ Layout ▸ Lock panels makes
tabs click-only (`Settings.panels_locked` replaces `panels_unlocked`, so everyone starts unlocked).
Window ▸ Layout ▸ Save as this page's default stores the arrangement on screen
(`Settings.page_defaults`); Reset page layout (also a page's right-click) comes back to it, and Reset
to built-in layout to the page's own. Undocked windows reopen where you left them
(`Layout.popped_rects`, saved once a moved window is still). MCP: `layout.page` takes the new names,
`layout.list` reports `locked` and `page_defaults`.

### Viewer surface (issue #80, simplify wave 1)
The Tools pane is gone: a slim icon rail floats over the Preview's left edge - Select, Crop (the crop
handles), Text, Shape ▾ and Mask ▾ (flyouts that remember the last pick), Draw - and a shape / draw
tool's options (fill, stroke, width, sides; brush, speed, record) sit in a strip along the viewer's
top edge only while that tool is active (the Mask tool's strip picks which mask its drag writes).
Show / Hide Tools shows or hides the rail; a saved layout with a Tools tab loads without it. Both
monitors have ONE transport row: click-to-type timecode, centred go to start / step / play / step /
go to end, then Zoom ▾ (Fit, 50, 100, 200 % of the project's pixels) and Fullscreen on the Preview,
Insert and Overwrite on the Source; "N dropped" shows only while frames drop, and a thin scrub line
is always under the picture (taller on hover). Everything else moved to right-click: on the Preview,
right-clicking a clip selects it and offers Reset Transform, Fill / Stretch to Frame, Crop Handles,
Add Mask, Add Text Here, then Stop, Mark In / Out / Clear, Zoom, Playback Resolution, Proxies, Movie
Mode, Social Guides, Scopes, Background and Snap to Canvas; on the Source, the marks, the smart edits
(with the nearest-cut readout), Source Tape, New Subclip, Playback Resolution and Close. A right-click
gives that monitor the transport keys, like a left click. The multicam angles are a compact list in
the viewer's top-right corner, and the rotate knob no longer shows the 4-arrow move cursor.

### Keys & actions (issue #84, simplify wave 1)
Every Action has a real group - File, Edit, Playback, Marking, Trimming, Timeline, Clip, Add, Audio,
Color, Captions, Viewer, Panels & Pages, Media, Tools, General (`hotkeys::GROUPS`; nothing lands in
"Other" any more) - shared by F1, Settings ▸ Hotkeys and `hotkeys.get`. The F1 sheet is searchable,
lists bound keys before unbound ones, adds "Keys outside the table" (Shift+S, Ctrl+Y, Backspace, Esc…)
and a "Mouse" section generated from the timeline's modifier table (`timeline::arm`), and shows the
palette's live chord instead of a hard-coded Ctrl+K. The command palette has one row per thing: a pane
with a Show / Hide Action is reached through that Action (wearing the pane's icon), the four pages are
their page Actions, arg-free tools read as words ("Focus (source)"), and read-only tools - whose reply
the palette never showed - are listed only in `:` mode. Rows honour the menus' icon overrides, say why
a disabled row is disabled, and the arrow keys scroll the list. Settings ▸ Hotkeys lines its columns up
across groups, and hovering a keymap preset lists what it changes. `UndoSettings` is gone (nothing
ever took the snapshot it restored).

### Library (issue #81, simplify 1)
The Library's header is one row: search · Filter ▾ · View ▾ · + Import, with a small "Project |
Browse" switch under it (was "Imported | Global"). Everything else is a right-click away: on bare
space (Import, Import from URL, New folder / sequence / adjustment layer, Link folder, Consolidate,
Remove unused (N), Sort, Columns, Clear recent, Open project), on an asset (Open in Source, Add to
timeline, Rename a subclip, Info…, Label, New subclip, Relink, Reveal, Regenerate proxy, Convert,
Compress, Remove - on the whole selection), on the column header (Columns + Sort). The bottom preview
and the selection strip are gone: a click shows the item in the Source monitor (paused), a
double-click plays it, and right-click ▸ Info… is where description, tags, label and folder are
edited. Sequences are listed again: drag one onto the timeline to nest it (with its audio twin),
double-click to open, right-click to rename or delete. Space over the Library is Play/Pause again
(it used to add the selection to the timeline; Enter still does); removing assets from the menu
toasts an Undo like Delete does.

### Pages (issue #78, simplify 0c)
The six top-right workspaces are four Resolve-style **pages** — Edit, Color, Audio, Export — in a
switcher centred in the menu bar (Alt+1..4; right-click a page ▸ Reset page layout). Each page is its
own arrangement and remembers it (`Settings.page` + `Settings.page_layouts`; `Settings.layout` stays
the page on screen); popped-out windows belong to the page they were popped on, and a switch clears
the layout history so Ctrl+Z can't put another page's arrangement back. A pre-pages settings file keeps
its old arrangement as the layout profile "Before update" and starts on the page its workspace maps to
(Simple/Edit/Text → Edit, Deliver → Export). Tab bars lose their four buttons for one `+` ("Add
panel": a menu of the panels not on screen, landing as that group's front tab); a tab's right-click
has Maximise (also double-click / backtick), Undock, Close, "Stay on this tab" (the old pin) and Set
icon. Panels are locked by default — tabs click, they don't drag (Window ▸ Layout ▸ Unlock panels);
dividers resize either way. The Timeline's tab is the sequence strip: "Main" plus the open sequence
with a × (a torn-off Timeline names the sequence in its title). Scopes and Export are panes
(`Pane::Scopes` reads the GPU stats only while drawn; Ctrl+E opens the Export page, whose Export
settings pane replaces the Export window). The menu bar is rebuilt (File / Edit / Clip / Timeline /
Playback / Window / Help) from `ui::menu` rows, niche panels under Window ▸ More; the menu-bar
timecode, the "editing: Main > X" breadcrumb, the View menu's Dynamic/Granular radios, Ctrl+Shift+G
and the welcome wizard's layout card are gone — following the selection is Settings ▸ General ▸
"Switch panel tabs to follow the selection". The Tools pane is on no page (still in Window ▸ More).
MCP: `layout.page`; `layout.workspace` still takes the old names; `layout.list` lists pages.

### Panels activate on use (issue #76, part C)
Opening a panel no longer changes behaviour or data: the Mixer shows a placeholder Main strip instead
of creating a bus, Auto-cut waits for Detect, the tracker box waits for Tracking ▸ "Place box", the
Subtitles Transcribe button runs through `App::transcribe_clip` (finished by `transcript_ctl::tick`
even when the pane is closed), and a hidden Source monitor no longer keeps Space/JKL
(`App::pane_drawn`, the set of panes drawn last frame). Auto-cut, Tracking, Transitions, the Settings
tabs, the Export window and the Subtitles sections now scroll.
- Tripwire count update: `pre_existing_repaint_sites_unchanged_and_named` drops from 14 to 13 - the
  Subtitles section's own transcribe-job poll is deleted with the job.

### Jobs pane (issue #64)
A dockable `Pane::Jobs` (View menu / `Ctrl+K` / the new menu-bar queue indicator; hidden by default
in every preset via `stack_unplaced`, like Source) lists every background task — the export slot and
render queue, bakes, converts, media/MCP jobs, the proxy build and the proxies still to build, URL
downloads, whisper/tracking/TTS, recordings, the waveform/thumbnail/pre-render caches, ffprobe — with
progress/ETA, Cancel where the job honours it (tts/tracking greyed with a tooltip; caches get one
whole-queue Clear), ▲▼ on the export queue and "Build next" on a queued proxy (`App.proxy_next`, via
the extracted pure `pick_next_proxy`). It replaces the four ad-hoc floating job windows
("Converting" / "Downloading" / "Rendering in place" / "Media"; `job_window` is deleted). Finished rows
stay in a 60 s "Recent" log. `Settings.jobs_auto_reveal` (default on) surfaces the tab through
`layout_ctl::surface` (Dynamic switches, Granular glows; a hidden tab is never re-opened — the
indicator covers that). MCP: `jobs.list/cancel/reorder/proxy_next` (`Ui`/`Read`, never undo).
Worker-shared cache mutexes are read with `try_lock` only (`assert`ed by
`try_lock_never_blocks_the_snapshot`).
- Tripwire count update: `pre_existing_repaint_sites_unchanged_and_named` drops from 15 to 14 —
  `job_window`'s one raw `request_repaint_after` (mod.rs) is deleted with the fn; `jobs_pane::tick`
  routes the same 150 ms cadence through `App::animate_until`, gated on a Running row and the pane or
  indicator having drawn that frame.
- Verification aid: `--screenshot` with `SE_JOBS_PANE=1` surfaces the Jobs tab before the shot.

The full UI/UX overhaul (`plans/ui-overhaul/`): 19 feature workstreams across waves 1-3 plus the
three wave-0 refactor PRs (22 total), closed out by this wave-4 docs pass (issue #38). This section folds what
used to be a separate `## unreleased` heading (PRs #12/#14, plus issues #16/#18/#21/#29/#32, kept
verbatim below) together with one new terse paragraph per remaining workstream area. Release exe at
wave-3-complete: **12,167,168 B (11.60 MB)**, sha `86c1793` — see `size_log.csv` and goals.md's
Core-goals line.

### Command palette & scripting (issue #20)
`Ctrl+K` opens a command palette over every Action, Pane, arg-free ToolDef (`:` opens a mini arg
form), script and workspace, with recency-ordered results; `F1` opens a keyboard-shortcuts cheat
sheet (a second palette entry point). Avid/Premiere/Resolve keymap presets are diff tables over the
defaults (no re-keying). Luau scripts gained `-- @name/@desc/@icon/@hotkey/@on <event>/@budget_ms`
header metadata so a script can appear as its own hotkey-bound, palette-visible command or subscribe
to an editor event.

### Trim & gesture primitives (issues #22, #23, #30)
A tiered `snap_target` (playhead > cursor > selected-clip edges > adjacent clips > markers) draws its
guide line before release and weakens as you zoom in. Ripple/roll/slip/slide/segment/multi-roller
trim primitives landed headless-first in `model/ops/trim.rs`, with `Track.locked`/`ripple`/`magnetic`
flags; `timeline-trim-gestures` then routed real mouse drags on the timeline through the frozen
`arm()` modifier table (Edge and Drop zones wired live; Body/Lane/Seam's non-Shift rows stay
tested-but-unwired, a known gap, not a silent claim of full coverage).

### Color engine (issue #24)
New `EffectKind::{Lut, Primaries, Qualifier, FrameBlend}` with GLSL bodies and CPU fallbacks, a
stdlib `.cube` 1D/3D LUT parser, builtin Looks, and master/source-clip effect chains via a new
`Asset.effects` field (applied ahead of any per-clip stack).

### Player rate/loop (issue #25)
`Clock.rate` and `Clock.loop_range` parameterise what used to be a fixed forward-1x, no-loop clock —
one mechanism now backs JKL shuttle (with a −1,−2,−4,−8 repeat ladder), Loop In→Out, Play In→Out,
Play Around Playhead, and the ±10-frame trim chords.

### Audio analysis & DSP (issues #19, #26)
Pure analysis functions over cached waveform peaks (onset/beat/BPM detection, peak/RMS/LUFS levels,
cross-correlation sync offset for multicam) back new AutoDuck/Normalize inspector buttons and
one-click repair chains; clips can be tagged with an `AudioRole` (Dialogue/Music/Sfx/Ambience).
`mixer_fx.rs` gained cascaded-notch DeHum, a lookahead-limiter Limiter, and a sidechain DeEsser, plus
a K-weighted LUFS meter fed to the UI over a ring buffer and per-bus gain automation keyframes.

### Canvas handles & alt-render monitor (issue #27)
On-canvas transform handles (corner/edge scale, rotate knob, a group box) replace numeric-field-only
transform editing, with canvas snap guides and viewer zoom/pan. Introduces the shared
`AltRenderState`/`AltRequest` async alt-render channel (GPU-accurate, never the CPU compositor,
paused whenever an export is running) that every later hover-preview/trim-view/Scopes/multicam
consumer builds on instead of its own render path.

### Export & delivery (issue #28)
One-click platform export-preset tiles (YouTube 1080p/4K, Shorts/Reels/TikTok 9:16, …), a render
queue with an ETA, Export In/Out range with letterbox, loudness normalisation, a render-in-place bake
pipeline (stabilize/denoise/slow-mo), and markers CSV / YouTube-chapters export and import.

### Layout modes & onboarding (issue #31)
`Settings.layout_mode` (Dynamic contextual vs. Granular explicit, chosen at first run and
toggleable later), named workspaces (Simple/Audio/Text/Deliver, `Alt+1`.. `6`), a per-panel pin/lock
so auto-surfacing can be opted out of per panel, a first-run welcome wizard and home screen, and an
adaptive tool strip.

### Inspector & Gallery (issue #33)
The inspector's clip-properties sections are now collapsible (`CollapsingState`, primary controls
first, fold state remembered in `Settings.inspector_folds`), with a new Color section for LUTs/
Primaries/Qualifier. `Pane::Presets` keeps its variant but now draws a tabbed Gallery (effects, node
graphs, adjustment layers, templates, LUTs, Looks, Titles, Captions) instead of the deleted
`presets_ui.rs`.

### Media library (issue #34)
Offline-media detection with a badge plus Relink/Consolidate, subclips and Smart Bins, sortable list
columns, thumbnail-viewport culling so large bins stay smooth, keyboard navigation, an empty-state
hint, and image-sequence import (with its own `import` `-- @on` hook call site).

### Source monitor (issue #32)
See "Source monitor" below — `Pane::Source` landed as part of this wave.

### Pro monitor, pro timeline & titles (issues #35, #36, #37)
Pro-monitor: a trim view (outgoing/incoming frames via two alt-render requests), dynamic trim,
Scopes, wipe compare, stills, and a 4-angle-capped multicam angle grid. Pro-timeline: asymmetric
multi-roller trims from the edit-point set, inline track header rename/colour/reorder, timeline view
presets, an overview minimap strip, and a Find window across clips/markers/text. Text-titles: a
Gallery ▸ Titles tab with a handful of built-in templates whose params are exposed as keyframeable
text animation (reveal/wave).

### Binary size, corrected (issue #18)
`size_log.csv`'s `bc4c19e` row corrects an earlier PR-body claim that the main-crate
`opt-level = "s"` step was skipped: the merged commit actually kept it (retroactively validated —
`headless_1000_clips_stays_fast` 0.93 ms, `bench_4k_preview` 29.0 ms/frame @720p / 74.7 ms/frame
@native-4K, no regression). Real measured delta: **15,970,816 B (15.23 MB)** at `aaa3fdc` →
**10,121,728 B (9.65 MB)** at `bc4c19e`, **-5,849,088 B (-5.58 MB)**. See the entry below for what
shipped in the diet itself, and goals.md for the post-feature-wave number.

The following entries were carried forward from this release's working `## unreleased` section
(PRs #12/#14, issues #16/#18/#21/#29/#32) rather than rewritten — they already match the density
this section aims for.

### Transcript & captions (issue #29)
- A transcribed clip's word timings now persist in the project (survive save/reopen) instead of
  dying when the Subtitles pane closed. A new collapsible **Transcript** section there: click a
  word to seek, drag-select a range and Delete to ripple-cut it (cues, markers and the transcript
  shift together), search across every transcribed clip, and an editable filler-word list
  ("um", "uh", "you know"…) with Mark-instead-first before it cuts.
- Karaoke-style captions: subtitles can now animate word-by-word against the transcript
  (Highlight / Pop / Typewriter), set from the new `subtitles.animation` script/MCP tool.
- "Get captions" is now a single button that names the exact download size before it fetches
  anything — never runs at startup.
- Any imported video/audio clip's right-click menu gained a **Transcript** submenu:
  Transcribe… (runs in the background, offers the model download first if needed), View
  transcript (a live, non-blocking word list with click-to-seek and a filter box), and Export
  transcript… (.txt / .srt / .json).
- Basic text-to-speech: a Speech panel in the Transcript section using Windows' own voices, which
  drops the result on the timeline as a linked audio clip.
- Scripts/agents can now drive all of this: `transcript.get/set/cut_words/remove_fillers/search/
  export`, `transcribe.run/install`, `tracking.run`, `subtitles.animation`, `tts.speak`, and
  `media.transcribe`/`media.transcript`.
### Source monitor (issue #32, source-monitor)
- `Pane::Source` is real: the old library preview (`lib_preview.rs`, deleted — it used to hijack the
  Preview pane) is now a dockable Source monitor (`ui/source_ui.rs` + `ui/app/source_pane.rs`) with its
  own `Player`, I/O marks (ticks + band on the shared `preview::scrub_bar`), a Source/Record focus
  button, a three-point / smart-edit button row (Splice, Overwrite, Append at End, Ripple Overwrite,
  Close Up, Place on Top + a nearest-cut readout), Subclip-from-marks and Source Tape (the bin laid end
  to end in a transient project — never touches the real project or its undo stack).
- Transport focus: Space/J/K/L/I/O/Home/End drive whichever of Source or Timeline was clicked last
  (the timeline is the fallback; a press on the Preview or Timeline pane hands focus back). The two
  players never run at once — starting the timeline pauses the source.
- `App::insert_at` is gone: every placement (drops, library "Add", voiceover/recording import,
  three-point edits) routes through `edit_ops::place`/`place_many` with an explicit `DropMode` —
  drop modifiers are live (none = Place, Ctrl = Splice, Alt = Overwrite, Shift = Place on Top), and
  an Alt-drop landing on a clip body is a Replace edit (`Project::replace_clip`: duration/effects/
  transform kept, linked audio swapped too), elsewhere a plain overwrite.
- New actions: Match Frame (F), Reveal in Library (Ctrl+Shift+R); Append at End / Ripple Overwrite /
  Close Up / Place on Top / Source Tape / Show-Hide Source Monitor (palette + Source pane buttons).
  `Project::subclip_from_marks`. MCP: `source.open/mark/get/focus/tape/insert/subclip`,
  `timeline.place`, `timeline.match_frame`, `timeline.smart_edit` (splice/overwrite/lift/extract/replace
  stay on trim-model's existing `timeline.*` tools — nothing re-registered).
- Tripwire count update: `pre_existing_repaint_sites_unchanged_and_named` drops from 16 to 15 —
  `lib_preview.rs`'s one raw `request_repaint_after` (the buffering spinner's 50 ms poll) is deleted with
  the file; `source_pane.rs` routes the same poll through `App::animate_until`, so this is a genuine
  migration of one of the tracked sites, not a relocation.
- Verification aid: `--screenshot` with `SE_SCREENSHOT_SOURCE=1` opens the project's first asset in the
  Source monitor with marks set before the shot (like `SE_SCREENSHOT_DELAY`).
- Deferred to same-day follow-ups per the plan's ownership notes: the `library.rs` edit (Source Tape
  filter toggle; Enter/double-click loads into Source) lands after media-library (#34); the timeline
  clip context-menu rows (Match Frame / Reveal in Library / Replace with Library Selection) after the
  wave-2 timeline owner merges — both verbs are already reachable by hotkey and palette.

### Forgiveness (issue #21)
- Toasts gained a kind (info/success/warn/error), an optional Undo button and an optional progress
  bar; `App::toast`/`toast_with_folder` (122 existing call sites) compile unchanged. Delete/Ripple
  Delete and Remove-unused-assets now toast an Undo.
- Every blocking Yes/No/Cancel dialog (6 sites: `confirm_discard`, both `act_overwrite` prompts,
  library's Clear recent/Remove unused, subtitles' Clear all/Import-replace) is now a non-blocking
  `egui::Window` (`src/ui/confirm.rs`) — nothing in the editor blocks the frame waiting on a click
  anymore. `confirm_discard() -> bool` is gone; `confirm_discard_then(on_yes)` is its non-blocking,
  continuation-based replacement.
- Debounced off-thread autosave (`Settings.autosave_secs`, 20 rolling backups per project under
  `%LOCALAPPDATA%\SimpleEditor\autosave`), a panic hook (`crash.log` + the latest autosave snapshot),
  and a non-blocking "Recover unsaved project?" offer on startup when a newer autosave exists.
  `App::fire_hook("project_open"/"project_save", ...)` gives `-- @on project_open`/`-- @on
  project_save` scripts a real call site (the other four `@on` events named in the plan are each a
  different workstream's own addition).
- Settings ▸ Performance gained a "Clear Caches" button + live on-disk cache size; History panel rows
  gained a Restore button (labeled "Restore history entry" in the undo stack).
- `Settings::load()` keeps its signature but now quarantines a corrupt `settings.json` to
  `settings.json.bad` instead of silently falling back to defaults over it; `App::new` toasts the
  reason via the new `Settings::load_reporting()`.
- Tripwire count update: `pre_existing_repaint_sites_unchanged_and_named` drops from 17 to 16 — the
  toast area's own `request_repaint_after` call moved out of `mod.rs` into the new `feedback.rs`
  (outside that test's scanned file list) as part of extracting the Toast type; the call itself still
  exists, just relocated, so the crate-wide site count is unchanged.

### Binary size (issue #18, size-diet)
- Dropped `egui_commonmark` (+ `egui_commonmark_backend`, `egui_extras`, `pulldown-cmark`) for a small
  in-house `ui::markdown` (~250 lines: headings, bold/italic/code spans, bullet lists, fenced code,
  links, `---` rules) — used by the Planner's Notes tab and the new "What's New" window.
- Dropped eframe's `default_fonts` feature (the embedded Hack/NotoEmoji/Ubuntu-Light/emoji-icon TTFs);
  `theme::fonts` now loads Segoe UI + Consolas straight from `%WINDIR%\Fonts`, falling back through
  Tahoma/Arial and Courier New/Lucida Console, and finally to `fontdb`'s system-font scan if none of
  those exist (never expected on Windows 10/11). Every headless UI test that builds a bare
  `egui::Context` now seeds it with `theme::test_fonts()` (`#[cfg(test)]`, zero release-binary cost) —
  without the Cargo feature, `egui::FontDefinitions::default()` is empty crate-wide, and several
  existing tests measure real text/button metrics.
- Dropped eframe's `persistence` feature (`ron`/`home`/egui's own memory serde); the window rect now
  lives in `Settings::window_rect`, written by a 500ms-debounced `winpos::tick` and seeded once at
  launch (`winpos::apply_rect`) from `main.rs`. Tradeoff: egui's own CollapsingHeader/scroll-position
  memory (e.g. the Planner's fold state) no longer survives a restart — accepted for wave 0, not
  rebuilt here (inspector fold state moves to `Settings` explicitly in wave-2 inspector-gallery).
- `[profile.release.package."*"] opt-level = "s"` for dependencies, plus the main-crate `opt-level =
  "s"` gate (kept, not reverted — see the "Binary size, corrected" entry above for the retroactive
  bench validation that confirms it shipped).
- Deleted verified-dead/duplicate code: `Tool::Zoom` (an unbound, always-no-op tool, including its
  `tool_drag` guard reference in `preview.rs`), `src/ui/presets_ui.rs` (270 lines — `Pane::Presets` now
  draws `library::reuse_ui`'s Effects/Node-graph/Adjustment-layer rows, the same ones `Pane::Library`'s
  Recent tab already used; Save-from-selection/rename/delete are gone from the UI until wave-2
  inspector-gallery's Gallery pane, restored meanwhile via the new `templates.save` MCP tool),
  `library.rs`'s `sequences_section` (zero call sites) and `templates_section` (test-only call site;
  its regression coverage moved to call `row()` directly), and the lib-preview mini-player's duplicated
  scrub-bar painting (now calls the one `preview::scrub_bar`, which returns a seek target instead of
  writing into `PreviewResponse` directly).
- New MCP tools: `help.changelog` (current version + this file's text, backing the What's New window)
  and `templates.save` (saves the given, or currently selected, clips as a reusable effect chain / node
  graph / clip template — keeps presets_ui.rs's deleted "Save from selection" capability scriptable).
- `scripts/size.ps1` (+ `size_log.csv`, `merge=union`) builds release and logs `<sha>,<bytes>,<note>`;
  `/se-verify` now runs it instead of a manual build-and-eyeball, and fails the pass on an unexplained
  >64 KB size growth. `--selftest` gained an `idle_repaint` step (a bare, idle egui frame requests no
  repaint) — smoke-level only, it cannot exercise the 17 pre-existing raw `ctx.request_repaint_after`
  call sites inside real panes' own live UI code (app.rs-descended files, `planner.rs`, `preview.rs`,
  `subtitles_ui.rs`) — those are a known, separately-tracked gap (`chore/migrate-repaint-timers`,
  unscheduled), not migrated by this PR. `App::animate_until` (added by #17/registries-schema-hooks) is
  therefore accurately the sanctioned path for **new** timed-repaint code from wave 0 onward, not yet a
  codebase-wide invariant — `winpos.rs`'s window-rect debounce and `whatsnew.rs`'s version-gate tick are
  its first two callers.
- Measured release exe: baseline `aaa3fdc` 15,970,816 B (15.23 MB) -> `bc4c19e` 10,121,728 B
  (9.65 MB), a delta of -5,849,088 B (-5.58 MB). See `size_log.csv`.

### Refactor (issue #16)
- Split the five largest files by responsibility, zero behaviour change: `model.rs` (6.1K lines) into
  `src/model/*.rs` (data types) + `src/model/ops/*.rs` (edit operations), `ui/app.rs` (6.8K lines)
  into `src/ui/app/*.rs`, `ui/timeline.rs` (4.3K lines) into `src/ui/timeline/*.rs`,
  `ui/settings_ui.rs` into `src/ui/settings_ui/*.rs`, and `ui/inspector.rs`'s clip-properties/text
  sections into `ui/inspector_audio.rs`/`ui/inspector_text.rs`. 657 tests unchanged, `--selftest`
  passes, release binary +46 KB (more per-file codegen boundaries after 5 files became 76 — not a
  behaviour change).

### Playback & performance (PR #14)
- **RAM-scaled playback cache**: `Settings::cache_mb` (Performance tab; 0 = automatic — ¼ of
  installed RAM, clamped 512 MB–4 GB) replaces the fixed 512 MB budget; lowering it frees RAM
  immediately. The prefetch horizon is sized from measured entry cost, so multi-layer GPU frames
  no longer oversubscribe it.
- **A finished proxy no longer restarts the read-ahead**: only the remapped source's spans (and
  decoders) are evicted — other clips keep their warm caches.
- **Decoded-source-frame cache** on the preview pool: scrubbing back past the composited cache is
  a memcpy instead of an ffmpeg re-seek/respawn.
- **Per-asset proxy status**: queued / building N % / ready — hourglass badge on library rows and
  tiles, a line in the inspector's Asset block, and the preview badge now names the file.

### Fixes (PR #12)
- Tool hotkeys match modifiers exactly (Shift+T/D/R work again); bare **M adds a marker at the
  playhead** (on the selected clip when under it), the Marker tool moved to Shift+M.
- "Use project background" on export actually works (CPU compositor honours the background;
  Overwrite-Original/MCP exports stay black unless opted in); background editable in project
  settings.
- History panel: layout rows can't desync the panel-undo stack, labels describe their own edit
  (derived lazily — no more per-gesture JSON parses), local-time day grouping, label cache
  invalidated on delete.
- Bulk edit clamps propagated keyframes into each sibling and never severs expression/path links;
  scale X/Y fully integrated (trim/split-safe keys, curve editor rows, ranges).
- Marker ruler scoped per sequence; snap-to-clip uses nearest edge; single-marker snap/link;
  row click seeks.
- Timer ticks (and notifies) with its tab hidden; moodboard tags typeable with one undo per
  gesture, in-app drags import, Import… button; text spans follow edits, style-paste clamps
  ranges, Clear Style on Selection; per-encoder quality scales (VP9/AV1 0–63, QSV floor 1) with
  honest warning bands; live shape preview uses the real tool style; Settings opens 900×700.

## beta-0.2.0

Nine PRs since beta-0.1.0.

### Playback & performance
- Deep read-ahead (~1.5 s prefetch horizon, ~0.5 s eviction-protected trail behind the playhead) and
  a real **buffering** state — the clock and audio pause together, a spinner shows, and playback
  resumes once the read-ahead refills, instead of audio racing ahead of frozen video.
- Background **proxy media**: an all-intra 720p proxy per imported clip, so scrubbing and reverse
  playback are as cheap as forward; exports always use the originals.
- Selective cache invalidation (an edit only evicts the frame ranges it can actually change), a
  DecoderPool LRU cap, and pre-opening the next clip's decoder before a cut.
- Fixed the library asset preview being permanently black (its canvas was never sized before the
  first frame arrived).
- Fixed space and timeline clicks fighting over playback focus: Space now controls whichever player
  owns the Preview pane, and clicking the timeline reclaims it from an open library preview.
- Fixed a bug where a focused numeric field could silently record/alter keyframes at the moving
  playhead when playback started.

### Library preview
- A hover-only click/drag scrub bar on the docked and fullscreen preview (previously only the
  library preview had one).
- A heartbeat-monitor style audio visualizer for audio-only files with no video and no cover art — a
  fixed EKG-blip silhouette that pulses with the current amplitude, reusing the same waveform peak
  data the timeline already computes. An embedded cover art image takes precedence when present, and
  it can be toggled from the preview's right-click menu.
- The library preview now works for files never imported into the project (Global/Recent), and gained
  the main preview's quality selector (100/75/50/25%) and buffering spinner. A still image previewed
  from the library shows just the picture — no transport controls.

### Editing
- **Edge transitions**: a transition can start/end a clip with nothing on the other side, blending
  from/to empty on both the CPU and GPU render paths.
- A transition's Start/End/Last/Next position selector, video opacity fades (not just audio), and
  transitions that select/multi-edit like clips on the timeline.
- **Live keyframe links** (After Effects–style): any keyframable property can follow a saved
  path or a Luau expression instead of being hand-keyed, re-evaluating from its source every frame.
  The expression field gets Luau syntax highlighting.
- One-click **aspect ratio / project-format presets** (YouTube, TikTok, Reels, Shorts, Square,
  Portrait, 4K…), quality/FPS quick buttons, user-saved project templates, and a toggleable
  **social-guide overlay** (platform safe zones and UI silhouettes) over the preview.

### Subtitles
- Regenerate cues from cached word timings without re-transcribing; a subtitle lane on the timeline
  (select, trim, split, resize like any clip); convert cues to editable Text clips; bulk panel actions;
  a whisper `--prompt` field; bold/italic/align/spacing/shadow and continuation-mark styling.

### UI & theming
- Cozy/Sharp look toggle (rounded corners and softened borders vs. the old flat look).
- 25 built-in theme presets, exportable/importable `.sedit-theme` files, per-clip-kind colour
  overrides, tab context menus (pop out/hide/set icon), layout presets, and an editor background
  image with tint/blur/opacity.
- No more white flash on startup (the window is created hidden and shown after its first frame).

## beta-0.1.0

Initial public beta — see the [release notes](https://github.com/KashTheKing/simple-editor/releases/tag/beta-0.1.0).
