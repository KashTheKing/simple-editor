//! ---- ws:quick-export ----
//! The top bar's Quick Export button (`Action::QuickExportAs`): a native Save As dialog with a file-type
//! dropdown, then the export starts straight away as a background job (Jobs pane), with the current
//! export settings or else the project's own. The result opens in the default media player.
//!
//! **Single footage** (`single_footage`): the project came from opening/importing ONE video file
//! directly (`Project.source_video`), that file is still the project's only asset, it is the only
//! media on the timeline (text, shapes, adjustment layers, captions and effects on top are fine),
//! there are no nested sequences, and the file is on disk. Then the dialog opens on that file's folder
//! and exact name, and saving over it - after an explicit "Replace <name>?" - runs the overwrite path:
//! render to a temp file beside it, release the decoders, swap it in with `swap_with_backup`, reopen.
//!
//! Normal (non-overwrite) quick exports remember their folder and type in `Project.quick_export`.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::settings::ExportPresetRef;

/// The dialog's file types, in dropdown order: (label, extension). Every one is a container the
/// exporter writes (`quick_types_match_the_exporter`).
pub(crate) const QUICK_TYPES: &[(&str, &str)] = &[
    ("MP4 video", "mp4"),
    ("MOV video", "mov"),
    ("MKV video", "mkv"),
    ("WebM video", "webm"),
    ("GIF", "gif"),
    ("WAV audio", "wav"),
    ("MP3 audio", "mp3"),
];

/// The project's single source video, when the project is "single footage" (see the module doc).
pub(crate) fn single_footage(p: &Project) -> Option<PathBuf> {
    let src = p.source_video.as_deref()?;
    let [a] = p.assets.as_slice() else { return None };
    let only_media = p.all_clips().all(|(_, c)| c.asset == 0 || c.asset == a.id);
    let ok = a.kind == ClipKind::Video
        && same_path(Path::new(&a.path), Path::new(src))
        && p.sequences.is_empty()
        && only_media
        && Path::new(src).is_file();
    ok.then(|| PathBuf::from(src))
}

/// Windows paths: case-insensitive, either slash.
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    n(a) == n(b)
}

/// What a quick export renders with: the current export settings (the last Export run, size/tile
/// included) when there are any, otherwise the project's own size; codec/quality come from Settings
/// (encoder "auto" resolves to `encoders::best_for(ext)` at export time). `ext` is the chosen type.
pub(crate) fn quick_as_target(settings: &Settings, ext: &str) -> ExportPresetRef {
    match &settings.last_export {
        Some(ExportPresetRef::Preset(name)) => ExportPresetRef::Preset(name.clone()),
        Some(ExportPresetRef::Custom { width, height, .. }) => {
            ExportPresetRef::Custom { ext: ext.into(), width: *width, height: *height }
        }
        None => ExportPresetRef::Custom { ext: ext.into(), width: 0, height: 0 },
    }
}

/// `<name>.bak` beside `original`.
pub(crate) fn bak_path(original: &Path) -> PathBuf {
    let mut s = original.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

/// Replace `original` with `temp` (same folder). The original is first kept as `<name>.bak` (a hard
/// link, or a copy where the volume has none), then `temp` is renamed over it in one atomic step
/// (retrying briefly: a killed decoder releases its handle a few ms late). Success deletes the
/// backup; on failure the original is untouched and the backup is kept.
pub(crate) fn swap_with_backup(temp: &Path, original: &Path) -> std::io::Result<()> {
    if !temp.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "the rendered file is missing"));
    }
    let bak = bak_path(original);
    let _ = std::fs::remove_file(&bak);
    if std::fs::hard_link(original, &bak).is_err() {
        std::fs::copy(original, &bak)?;
    }
    let mut r = std::fs::rename(temp, original);
    let deadline = Instant::now() + Duration::from_millis(500);
    while r.is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        r = std::fs::rename(temp, original);
    }
    if r.is_ok() {
        let _ = std::fs::remove_file(&bak);
    }
    r
}

/// Open a finished file in the default app for its type (the user's media player).
pub(super) fn open_in_player(path: &Path) {
    // explorer opens Documents instead for a forward-slash path
    let p = path.to_string_lossy().replace('/', "\\");
    let _ = std::process::Command::new("explorer").arg(p).spawn();
}

impl App {
    /// The Quick Export button / `Action::QuickExportAs`. `avoid_source`: "Choose another name" after
    /// declining a replace - the dialog then suggests `<stem>_edit.<ext>` instead of the original.
    pub(super) fn act_quick_export_as(&mut self, avoid_source: bool) {
        if self.timeline_is_empty() {
            self.toast("Nothing to export - the timeline is empty");
            return;
        }
        if self.ffmpeg_missing() {
            return;
        }
        let single = single_footage(&self.project);
        let remembered = self.project.quick_export.clone();
        let (dir, stem, ext) = match (&single, remembered) {
            (Some(src), _) => {
                let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "mp4".into());
                let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                let stem = if avoid_source { format!("{stem}_edit") } else { stem };
                (src.parent().map(Path::to_path_buf), stem, ext)
            }
            (None, Some((dir, ext))) => (Some(PathBuf::from(dir)), self.project.name.clone(), ext),
            (None, None) => (Some(tools_export::default_out_dir(self)), self.project.name.clone(), "mp4".into()),
        };
        // the remembered / source type leads the dropdown, so it is the one selected
        let mut d = rfd::FileDialog::new().set_title("Quick export");
        for (label, e) in QUICK_TYPES.iter().filter(|(_, e)| *e == ext).chain(QUICK_TYPES.iter().filter(|(_, e)| *e != ext)) {
            d = d.add_filter(*label, &[*e]);
        }
        if !QUICK_TYPES.iter().any(|(_, e)| *e == ext) {
            d = d.add_filter(format!("{} (same as the original)", ext.to_uppercase()), &[ext.as_str()]);
        }
        d = d.set_file_name(format!("{stem}.{ext}"));
        if let Some(dir) = dir.filter(|d| d.is_dir()) {
            d = d.set_directory(dir);
        }
        let Some(mut out) = d.save_file() else { return };
        if out.extension().is_none() {
            out.set_extension(&ext);
        }
        self.quick_export_to(out, single);
    }

    /// The dialog's answer. Over the single-footage original → ask first (Replace / Choose another
    /// name); never overwrites without that Yes.
    pub(super) fn quick_export_to(&mut self, out: PathBuf, single: Option<PathBuf>) {
        if let Some(src) = single.filter(|s| same_path(s, &out)) {
            let name = src.file_name().unwrap_or_default().to_string_lossy().into_owned();
            confirm::ask_choice(
                format!("Replace {name}?"),
                format!("The original will be overwritten:\n{}", src.display()),
                ("Replace", "Choose another name…"),
                move |app| app.quick_overwrite(src),
                |app| app.act_quick_export_as(true),
            );
            return;
        }
        if files::refuses_source(&self.project, &out) {
            self.toast("That file is used by this project - pick another name");
            return;
        }
        let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let target = quick_as_target(&self.settings, &ext);
        match tools_export::quick_export(self, Some(target), Some(out.clone())) {
            Ok(_) => {
                if let Some(dir) = out.parent() {
                    self.project.quick_export = Some((dir.to_string_lossy().into_owned(), ext));
                }
                self.quick_open = Some(out);
            }
            Err(e) => self.toast(e),
        }
    }

    /// Confirmed: re-encode over the original (Overwrite path: temp beside it, release, swap, reopen).
    fn quick_overwrite(&mut self, src: PathBuf) {
        if self.export.is_some() {
            self.toast("An export is running - try again when it finishes");
            return;
        }
        self.quick_open = Some(src.clone());
        self.act_overwrite_offer_save_first(src.to_string_lossy().into_owned());
    }
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    if a != Action::QuickExportAs {
        return false;
    }
    app.act_quick_export_as(false);
    true
}

/// Top bar: the Quick Export button (right side, beside the Jobs indicator).
pub(super) fn button(app: &mut App, ui: &mut egui::Ui) {
    let r = tools::icon_button(ui, &app.palette, egui::Id::new("se-quick-export"), tools::Glyph::Upload, "Quick export", false);
    if r.clicked() {
        app.act(Action::QuickExportAs);
    }
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "export.quick_file",
    desc: "Quick Export to a file without the dialog: current export settings, else project settings; type from the extension. Overwriting the project's single source video needs replace=true.",
    args: &["path:string:true:output file", "replace:boolean:false:allow overwriting the single-footage original"],
    kind: ToolKind::Job,
    run: |app, args| {
        let out = PathBuf::from(req(arg_str(args, "path"), "path")?);
        if let Some(src) = single_footage(&app.project).filter(|s| same_path(s, &out)) {
            if !args.get("replace").and_then(Value::as_bool).unwrap_or(false) {
                return Err(format!("{} is the original - pass replace=true to overwrite it", src.display()));
            }
            app.quick_overwrite(src.clone());
            return match &app.export {
                Some((p, _)) => Ok(ToolOutcome::Job(p.clone(), src)),
                None => Err("the overwrite did not start (a save prompt may be waiting)".into()),
            };
        }
        if files::refuses_source(&app.project, &out) {
            return Err("that file is used by this project".into());
        }
        let ext = out.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let target = quick_as_target(&app.settings, &ext);
        match tools_export::quick_export(app, Some(target), Some(out.clone()))? {
            Some((p, o)) => {
                app.quick_open = Some(o.clone());
                Ok(ToolOutcome::Job(p, o))
            }
            None => Err("export was queued instead of started".into()),
        }
    },
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::export::{codec_args, AUDIO_EXTS};
    use crate::model::{Asset, Clip, Cue};

    fn media(path: &Path, kind: ClipKind) -> Asset {
        Asset {
            id: Id::default(),
            path: path.to_string_lossy().into_owned(),
            kind,
            duration: 4.0,
            width: 640,
            height: 360,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        }
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-quick-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn opened(path: &Path) -> Project {
        std::fs::write(path, b"video").unwrap();
        Project::from_media(media(path, ClipKind::Video))
    }

    #[test]
    fn single_footage_true_for_an_opened_video_with_text_on_top() {
        let d = tmp_dir("single");
        let src = d.join("clip.mp4");
        let mut p = opened(&src);
        assert_eq!(single_footage(&p).as_deref(), Some(src.as_path()));
        // a caption cue and a text clip on top keep it single footage
        p.subtitles.push(Cue { id: 1, start: 0.0, end: 1.0, text: "hi".into() });
        let id = p.new_id();
        p.tracks[0].clips.push(Clip::new(id, ClipKind::Text, "title", 0.0, 1.0));
        assert!(single_footage(&p).is_some());
    }

    #[test]
    fn single_footage_false_cases() {
        let d = tmp_dir("notsingle");
        let src = d.join("clip.mp4");
        // a new project, no source
        assert!(single_footage(&Project::new()).is_none());
        // a second asset imported
        let mut p = opened(&src);
        p.add_asset(media(&d.join("other.mp4"), ClipKind::Video));
        assert!(single_footage(&p).is_none());
        // the source file is gone
        let p = opened(&src);
        std::fs::remove_file(&src).unwrap();
        assert!(single_footage(&p).is_none());
        // an opened audio file is not footage to overwrite
        let song = d.join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        assert!(single_footage(&Project::from_media(media(&song, ClipKind::Audio))).is_none());
    }

    #[test]
    fn settings_fall_back_from_export_settings_to_project_settings() {
        let mut s = Settings::default();
        s.last_export = None;
        // no export run yet → project size (0×0), the chosen type
        assert_eq!(quick_as_target(&s, "webm"), ExportPresetRef::Custom { ext: "webm".into(), width: 0, height: 0 });
        // the Export page's last custom size wins, the chosen type still overrides its container
        s.last_export = Some(ExportPresetRef::Custom { ext: "mp4".into(), width: 1280, height: 720 });
        assert_eq!(quick_as_target(&s, "mov"), ExportPresetRef::Custom { ext: "mov".into(), width: 1280, height: 720 });
        // a platform tile stays that tile
        s.last_export = Some(ExportPresetRef::Preset("YouTube".into()));
        assert_eq!(quick_as_target(&s, "mp4"), ExportPresetRef::Preset("YouTube".into()));
    }

    #[test]
    fn quick_types_match_the_exporter() {
        let exts: Vec<&str> = QUICK_TYPES.iter().map(|(_, e)| *e).collect();
        for want in ["mp4", "mov", "webm", "gif", "wav", "mp3"] {
            assert!(exts.contains(&want), "{want} missing from the dialog");
        }
        for e in exts {
            let args = codec_args(e, "auto", 20, "", &[]).join(" ");
            if AUDIO_EXTS.contains(&e) {
                assert!(args.contains("-c:a") && !args.contains("-c:v"), "{e}: {args}");
            } else {
                assert!(args.contains("-c:v"), "{e} is not a video container the exporter writes: {args}");
            }
        }
    }

    #[test]
    fn overwrite_swaps_atomically_and_drops_the_backup() {
        let d = tmp_dir("swap");
        let (orig, temp) = (d.join("clip.mp4"), d.join(".clip.tmp.mp4"));
        std::fs::write(&orig, b"old").unwrap();
        std::fs::write(&temp, b"new").unwrap();
        swap_with_backup(&temp, &orig).unwrap();
        assert_eq!(std::fs::read(&orig).unwrap(), b"new".to_vec());
        assert!(!temp.exists() && !bak_path(&orig).exists());
    }

    #[test]
    fn overwrite_failure_keeps_the_original_and_its_backup() {
        let d = tmp_dir("swapfail");
        let orig = d.join("clip.mp4");
        std::fs::write(&orig, b"old").unwrap();
        // the rendered temp is a directory: the rename fails
        let temp = d.join(".clip.tmp.mp4");
        std::fs::create_dir(&temp).unwrap();
        std::fs::write(temp.join("x"), b"x").unwrap();
        assert!(swap_with_backup(&temp, &orig).is_err());
        assert_eq!(std::fs::read(&orig).unwrap(), b"old".to_vec());
        // a missing render also leaves everything alone
        assert!(swap_with_backup(&d.join("nope.mp4"), &orig).is_err());
        assert_eq!(std::fs::read(&orig).unwrap(), b"old".to_vec());
    }

    #[test]
    fn overwrite_failure_mid_swap_keeps_a_backup() {
        let d = tmp_dir("swaplock");
        let (orig, temp) = (d.join("clip.mp4"), d.join(".clip.tmp.mp4"));
        std::fs::write(&orig, b"old").unwrap();
        std::fs::write(&temp, b"new").unwrap();
        // the original held open without delete sharing (a decoder that was not released): the swap fails
        use std::os::windows::fs::OpenOptionsExt;
        let held = std::fs::OpenOptions::new().read(true).share_mode(1 /* FILE_SHARE_READ */).open(&orig).unwrap();
        assert!(swap_with_backup(&temp, &orig).is_err());
        drop(held);
        assert_eq!(std::fs::read(&orig).unwrap(), b"old".to_vec());
        assert_eq!(std::fs::read(bak_path(&orig)).unwrap(), b"old".to_vec(), "the backup stays after a failed swap");
    }

    #[test]
    fn replacing_the_original_requires_the_confirmation() {
        // the dialog's answer over the original never starts an export by itself: it stages the
        // Replace / Choose another name window, and only its Yes calls `quick_overwrite`
        let src = include_str!("quick_export.rs");
        let body = &src[src.find("fn quick_export_to(").unwrap()..src.find("fn quick_overwrite(").unwrap()];
        let ask = body.find("confirm::ask_choice(").expect("the replace must be asked");
        let ret = body.find("return;").unwrap();
        assert!(ask < ret, "the confirm comes before anything else on the overwrite branch");
        assert_eq!(body.matches("quick_overwrite(").count(), 1, "only the confirm's Yes overwrites");
        assert!(body[ask..ret].contains("move |app| app.quick_overwrite(src)"));
        // and the MCP row needs replace=true
        let tool = &src[src.find("name: \"export.quick_file\"").unwrap()..];
        assert!(tool.find("\"replace\"").unwrap() < tool.find("app.quick_overwrite").unwrap());
    }

    #[test]
    fn the_button_triggers_the_action() {
        assert_eq!(Action::from_id("quick_export_as"), Some(Action::QuickExportAs));
        let src = include_str!("quick_export.rs");
        let body = &src[src.find("pub(super) fn button(").unwrap()..src.find("pub const TOOLS").unwrap()];
        assert!(body.contains("app.act(Action::QuickExportAs)"));
        assert!(body.contains("\"Quick export\""));
        let menus = include_str!("menus.rs");
        assert!(menus.contains("quick_export::button(self, ui)"), "the top bar draws the button");
        // and the action reaches act_quick_export_as through ACT_HANDLERS
        assert!(include_str!("mod.rs").contains("quick_export::act,"));
        let act = &src[src.find("pub(super) fn act(").unwrap()..src.find("pub(super) fn button(").unwrap()];
        assert!(act.contains("app.act_quick_export_as(false)"));
    }
}
