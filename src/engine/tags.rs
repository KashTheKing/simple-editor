//! ---- ws:music-burn ----
//! Media tags (title, artist, album, ... and the cover picture) read with ffprobe and written back with
//! ffmpeg as a stream copy: nothing is re-encoded. A write goes to a temporary file beside the original,
//! the original is renamed to `<file>.bak`, the new file takes its name, and the `.bak` is deleted only
//! once the new file probes with the same streams and length. Any failure puts the original back.

use crate::media::ffpipe;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// One editable field: its ffmpeg key and the label the Inspector shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    /// Shown for music (audio files).
    pub audio: bool,
    /// Shown for video files.
    pub video: bool,
    /// Several lines (comment, lyrics, description).
    pub multiline: bool,
}

const fn f(key: &'static str, label: &'static str, audio: bool, video: bool, multiline: bool) -> Field {
    Field { key, label, audio, video, multiline }
}

/// Every field, in Inspector order.
pub const FIELDS: &[Field] = &[
    f("title", "Title", true, true, false),
    f("artist", "Artist", true, false, false),
    f("album", "Album", true, false, false),
    f("album_artist", "Album artist", true, false, false),
    f("genre", "Genre", true, true, false),
    f("date", "Year", true, false, false),
    f("track", "Track", true, false, false),
    f("disc", "Disc", true, false, false),
    f("description", "Description", false, true, true),
    f("rating", "Rating", false, true, false),
    f("comment", "Comment", true, true, true),
    f("lyrics", "Lyrics", true, false, true),
];

pub fn field(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.key == key)
}

/// Tags read from a file. `values` only holds the keys in `FIELDS` (other tags are kept on write by
/// `-map_metadata 0` and never shown).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaTags {
    pub values: BTreeMap<String, String>,
    /// The cover picture's stream index in the file (an attached picture), if any.
    pub cover: Option<usize>,
    /// The cover's codec (`mjpeg`, `png`, ...): the extension to extract it as.
    pub cover_codec: String,
    /// Video streams that are not covers (a music file has none).
    pub video_streams: usize,
    pub audio_streams: usize,
    pub duration: f64,
}

impl MediaTags {
    pub fn get(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }
}

/// What a write does with the cover picture.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum CoverEdit {
    #[default]
    Keep,
    Remove,
    /// A picture file (jpg / png, others are converted to png first).
    Set(PathBuf),
}

/// One write: `set` holds new values (an empty value removes the tag); keys not in it are left alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TagEdit {
    pub set: BTreeMap<String, String>,
    pub cover: CoverEdit,
}

impl TagEdit {
    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.cover == CoverEdit::Keep
    }
}

/// Alternative spellings ffprobe reports for our keys (ID3, Vorbis comments, iTunes atoms, RIFF INFO).
fn canonical(key: &str) -> Option<&'static str> {
    let k = key.to_ascii_lowercase();
    Some(match k.as_str() {
        "title" => "title",
        "artist" => "artist",
        "album" => "album",
        "album_artist" | "albumartist" | "album artist" => "album_artist",
        "genre" => "genre",
        "date" | "year" | "creation_date" => "date",
        "track" | "tracknumber" => "track",
        "disc" | "discnumber" => "disc",
        "comment" => "comment",
        "description" | "desc" | "synopsis" => "description",
        "rating" => "rating",
        _ if k == "lyrics" || k.starts_with("lyrics-") || k == "unsyncedlyrics" || k == "uslt" => "lyrics",
        _ => return None,
    })
}

/// Parse `ffprobe -print_format json -show_format -show_streams` output.
pub fn parse_probe(v: &Value) -> MediaTags {
    let mut t = MediaTags::default();
    let mut put = |tags: Option<&Value>, t: &mut MediaTags| {
        if let Some(obj) = tags.and_then(Value::as_object) {
            for (k, val) in obj {
                if let (Some(c), Some(s)) = (canonical(k), val.as_str()) {
                    t.values.entry(c.to_string()).or_insert_with(|| s.trim_end_matches('\0').to_string());
                }
            }
        }
    };
    put(v.pointer("/format/tags"), &mut t);
    for s in v.get("streams").and_then(Value::as_array).into_iter().flatten() {
        let idx = s.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let pic = s.pointer("/disposition/attached_pic").and_then(Value::as_i64).unwrap_or(0) != 0;
        match s.get("codec_type").and_then(Value::as_str) {
            Some("video") if pic => {
                if t.cover.is_none() {
                    t.cover = Some(idx);
                    t.cover_codec = s.get("codec_name").and_then(Value::as_str).unwrap_or("").to_string();
                }
            }
            Some("video") => t.video_streams += 1,
            Some("audio") => {
                t.audio_streams += 1;
                // Ogg / Opus keep their comments on the stream
                put(s.get("tags"), &mut t);
            }
            _ => {}
        }
    }
    t.duration = v
        .pointer("/format/duration")
        .and_then(|d| d.as_str().and_then(|s| s.parse().ok()).or_else(|| d.as_f64()))
        .unwrap_or(0.0);
    t
}

fn ffprobe_json(path: &Path) -> Result<Value, String> {
    let exe = ffpipe::ffprobe_exe().ok_or("ffprobe.exe not found")?;
    let out = ffpipe::command(&exe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(format!("can't read {}: {}", path.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe output: {e}"))
}

/// Read the tags of `path`.
pub fn read(path: &Path) -> Result<MediaTags, String> {
    Ok(parse_probe(&ffprobe_json(path)?))
}

/// Which container a file is, by extension: decides which tags and covers it can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Mp3,
    Mp4,
    Flac,
    Matroska,
    Ogg,
    Wav,
    Other,
}

pub fn container(path: &Path) -> Container {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => Container::Mp3,
        "mp4" | "m4a" | "m4v" | "mov" | "m4b" | "3gp" => Container::Mp4,
        "flac" => Container::Flac,
        "mkv" | "mka" | "webm" => Container::Matroska,
        "ogg" | "opus" | "oga" => Container::Ogg,
        "wav" => Container::Wav,
        _ => Container::Other,
    }
}

impl Container {
    pub fn can_cover(self) -> bool {
        matches!(self, Container::Mp3 | Container::Mp4 | Container::Flac)
    }
    /// Does this container keep `key`? (`.mp4` has no rating atom; FLAC folds description into comment.)
    pub fn keeps(self, key: &str) -> bool {
        match (self, key) {
            (Container::Mp4, "rating") => false,
            (Container::Flac | Container::Ogg, "description") => false,
            (Container::Wav, "lyrics" | "rating" | "description" | "album_artist" | "disc") => false,
            _ => true,
        }
    }
}

/// The ffmpeg arguments (after `-i path [-i cover]`) for `edit` on a file whose tags are `cur`.
/// Pure, so the mapping is testable without running ffmpeg.
pub fn write_args(c: Container, cur: &MediaTags, edit: &TagEdit, cover_input: bool) -> Result<Vec<String>, String> {
    let mut a: Vec<String> = Vec::new();
    let s = |x: &str| x.to_string();
    let new_cover = matches!(edit.cover, CoverEdit::Set(_));
    if new_cover && !c.can_cover() {
        return Err("this file type can't hold a cover picture (use mp3, m4a, mp4 or flac)".into());
    }
    if edit.cover == CoverEdit::Keep {
        a.extend([s("-map"), s("0")]);
    } else {
        // `V` = video that is not an attached picture: every old cover goes
        for m in ["0:V?", "0:a?", "0:s?"] {
            a.extend([s("-map"), s(m)]);
        }
    }
    if new_cover && cover_input {
        a.extend([s("-map"), s("1:v:0")]);
    }
    a.extend([s("-c"), s("copy"), s("-map_metadata"), s("0")]);
    if new_cover && cover_input {
        let n = cur.video_streams; // the cover's index among the output's video streams
        a.extend([format!("-disposition:v:{n}"), s("attached_pic")]);
        if matches!(c, Container::Mp3 | Container::Flac) {
            a.extend([format!("-metadata:s:v:{n}"), s("title=Album cover")]);
            a.extend([format!("-metadata:s:v:{n}"), s("comment=Cover (front)")]);
        }
    }
    if c == Container::Mp3 {
        a.extend([s("-id3v2_version"), s("3"), s("-write_id3v1"), s("1")]);
    }
    for (k, v) in &edit.set {
        if field(k).is_none() {
            return Err(format!("unknown tag '{k}' (have: {})", FIELDS.iter().map(|f| f.key).collect::<Vec<_>>().join(", ")));
        }
        if !c.keeps(k) {
            if v.is_empty() {
                continue;
            }
            return Err(format!("{} files can't hold the {} tag", ext_name(c), field(k).map_or(k.as_str(), |f| f.label)));
        }
        // lyrics are "lyrics-XXX" in some files: clear every spelling we read back
        a.extend([s("-metadata"), format!("{k}={v}")]);
        if c == Container::Mp4 && k == "description" {
            a.extend([s("-metadata"), format!("synopsis={v}")]);
        }
    }
    Ok(a)
}

fn ext_name(c: Container) -> &'static str {
    match c {
        Container::Mp3 => "MP3",
        Container::Mp4 => "MP4",
        Container::Flac => "FLAC",
        Container::Matroska => "MKV",
        Container::Ogg => "Ogg",
        Container::Wav => "WAV",
        Container::Other => "These",
    }
}

fn run_ffmpeg(args: &[std::ffi::OsString]) -> Result<(), String> {
    let exe = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let out = ffpipe::command(&exe)
        .args(["-nostdin", "-hide_banner", "-v", "error", "-y"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed").trim().to_string())
    }
}

/// A picture ffmpeg can copy into any container: jpg / png as they are, anything else converted to png
/// in the temp folder.
fn cover_source(p: &Path) -> Result<PathBuf, String> {
    if !p.is_file() {
        return Err(format!("no picture at {}", p.display()));
    }
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if matches!(ext.as_str(), "jpg" | "jpeg" | "png") {
        return Ok(p.to_path_buf());
    }
    let out = std::env::temp_dir().join(format!("se-cover-{}.png", std::process::id()));
    run_ffmpeg(&["-i".into(), p.into(), "-frames:v".into(), "1".into(), out.clone().into()])?;
    Ok(out)
}

/// `<dir>/<stem>.se-tags.<ext>`: beside the original (same drive, so the swap is a rename) with the same
/// extension (ffmpeg picks the muxer from it).
fn temp_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("media");
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    path.with_file_name(format!("{stem}.se-tags.{ext}"))
}

pub fn bak_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
}

/// Write `edit` into `path` (stream copy, temp file, `.bak` swap). Returns the tags read back.
pub fn write(path: &Path, edit: &TagEdit) -> Result<MediaTags, String> {
    if edit.is_empty() {
        return read(path);
    }
    let cur = read(path)?;
    let c = container(path);
    let cover = match &edit.cover {
        CoverEdit::Set(p) => Some(cover_source(p)?),
        _ => None,
    };
    let mut args: Vec<std::ffi::OsString> = vec!["-i".into(), path.into()];
    if let Some(cv) = &cover {
        args.extend(["-i".into(), cv.into()]);
    }
    args.extend(write_args(c, &cur, edit, cover.is_some())?.into_iter().map(Into::into));
    let tmp = temp_path(path);
    args.push(tmp.clone().into());
    if let Err(e) = run_ffmpeg(&args) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // the new file must hold the same media before it replaces the old one
    let new = match read(&tmp) {
        Ok(n) if n.audio_streams == cur.audio_streams && n.video_streams == cur.video_streams && (n.duration - cur.duration).abs() < 0.5 => n,
        Ok(_) => {
            let _ = std::fs::remove_file(&tmp);
            return Err("the rewritten file did not match the original; nothing was changed".into());
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    replace(path, &tmp)?;
    Ok(new)
}

/// original -> `.bak`, temp -> original, then drop the `.bak`. A failed step puts the original back.
pub fn replace(path: &Path, tmp: &Path) -> Result<(), String> {
    let bak = bak_path(path);
    let _ = std::fs::remove_file(&bak);
    if let Err(e) = std::fs::rename(path, &bak) {
        let _ = std::fs::remove_file(tmp);
        return Err(in_use(path, e));
    }
    if let Err(e) = std::fs::rename(tmp, path) {
        let _ = std::fs::rename(&bak, path);
        let _ = std::fs::remove_file(tmp);
        return Err(in_use(path, e));
    }
    let _ = std::fs::remove_file(&bak);
    Ok(())
}

fn in_use(path: &Path, e: std::io::Error) -> String {
    if e.raw_os_error() == Some(32) {
        format!("{} is open in another program; close it and save again", path.display())
    } else {
        format!("can't replace {}: {e}", path.display())
    }
}

/// Save the cover picture of `path` to `out_dir/<name>.<jpg|png>`; None when it has none.
pub fn extract_cover(path: &Path, out_dir: &Path) -> Result<Option<PathBuf>, String> {
    let t = read(path)?;
    let Some(idx) = t.cover else { return Ok(None) };
    let ext = if t.cover_codec == "png" { "png" } else { "jpg" };
    let _ = std::fs::create_dir_all(out_dir);
    let out = out_dir.join(format!("{}.{ext}", cover_key(path)));
    let codec = if matches!(t.cover_codec.as_str(), "png" | "mjpeg") { "copy" } else { "png" };
    run_ffmpeg(&[
        "-i".into(),
        path.into(),
        "-map".into(),
        format!("0:{idx}").into(),
        "-frames:v".into(),
        "1".into(),
        "-c".into(),
        codec.into(),
        out.clone().into(),
    ])?;
    Ok(Some(out))
}

/// A video frame at `t` seconds as a jpg (Inspector ▸ Cover ▸ From video frame).
pub fn grab_frame(path: &Path, t: f64, out: &Path) -> Result<(), String> {
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    run_ffmpeg(&[
        "-ss".into(),
        format!("{:.3}", t.max(0.0)).into(),
        "-i".into(),
        path.into(),
        "-frames:v".into(),
        "1".into(),
        "-q:v".into(),
        "2".into(),
        out.into(),
    ])
}

/// A stable short name for a file's cached cover (path + modified time).
pub fn cover_key(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    if let Ok(m) = std::fs::metadata(path).and_then(|m| m.modified()) {
        m.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// The cover cache folder.
pub fn covers_dir() -> PathBuf {
    crate::settings::Settings::cache_dir().join("covers")
}

/// The values several files share, per field: Some(value) when all agree, None = "Mixed".
pub fn merged(all: &[&MediaTags]) -> BTreeMap<&'static str, Option<String>> {
    let mut out = BTreeMap::new();
    for fd in FIELDS {
        let mut vals = all.iter().map(|t| t.get(fd.key));
        let first = vals.next().unwrap_or("");
        let same = vals.all(|v| v == first);
        out.insert(fd.key, same.then(|| first.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_ffmpeg() -> bool {
        ffpipe::ffmpeg_exe().is_some() && ffpipe::ffprobe_exe().is_some()
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-tags-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A 1 s generated file of the given kind.
    pub(crate) fn make(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        let ext = p.extension().unwrap().to_str().unwrap().to_string();
        let mut a: Vec<std::ffi::OsString> = vec!["-f".into(), "lavfi".into(), "-i".into(), "sine=f=440:d=1".into()];
        if ext == "mp4" {
            a.extend(["-f".into(), "lavfi".into(), "-i".into(), "testsrc=d=1:s=160x120:r=10".into()]);
            a.extend(["-c:v".into(), "libx264".into(), "-pix_fmt".into(), "yuv420p".into()]);
        }
        if matches!(ext.as_str(), "mp4" | "m4a") {
            a.extend(["-c:a".into(), "aac".into()]);
        }
        a.push(p.clone().into());
        run_ffmpeg(&a).unwrap();
        p
    }

    fn picture(dir: &Path) -> PathBuf {
        let p = dir.join("cover.png");
        run_ffmpeg(&["-f".into(), "lavfi".into(), "-i".into(), "color=red:s=32x32:d=1".into(), "-frames:v".into(), "1".into(), p.clone().into()])
            .unwrap();
        p
    }

    #[test]
    fn tags_and_cover_round_trip_on_mp3_m4a_flac_mp4() {
        if !have_ffmpeg() {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let d = tmpdir("rt");
        let pic = picture(&d);
        for name in ["a.mp3", "a.m4a", "a.flac", "a.mp4"] {
            let p = make(&d, name);
            let video = name.ends_with(".mp4");
            let mut e = TagEdit::default();
            for (k, v) in [("title", "Song é"), ("artist", "Artist"), ("album", "Album"), ("genre", "Synthwave"), ("date", "2024"), ("track", "3/10"), ("comment", "hi")] {
                e.set.insert(k.into(), v.into());
            }
            if !video {
                e.set.insert("album_artist".into(), "Various".into());
                e.set.insert("disc".into(), "1/2".into());
                e.set.insert("lyrics".into(), "la la la".into());
            } else {
                e.set.insert("description".into(), "A video".into());
            }
            e.cover = CoverEdit::Set(pic.clone());
            let back = write(&p, &e).unwrap_or_else(|err| panic!("{name}: {err}"));
            for (k, v) in &e.set {
                assert_eq!(back.get(k), v.as_str(), "{name}: {k}");
            }
            assert!(back.cover.is_some(), "{name}: cover written");
            assert_eq!(back.video_streams, video as usize, "{name}: the cover is not counted as video");
            assert!(!bak_path(&p).exists(), "{name}: .bak removed after success");
            assert!(!temp_path(&p).exists(), "{name}: temp removed");
            // the cover comes back out as a picture
            let out = extract_cover(&p, &d.join("covers")).unwrap().expect("a cover");
            assert!(std::fs::metadata(&out).unwrap().len() > 0, "{name}");
            // edit one tag, clear another, keep the cover
            let mut e2 = TagEdit::default();
            e2.set.insert("title".into(), "Second".into());
            e2.set.insert("comment".into(), String::new());
            let back = write(&p, &e2).unwrap();
            assert_eq!(back.get("title"), "Second", "{name}");
            assert_eq!(back.get("comment"), "", "{name}: cleared");
            assert_eq!(back.get("artist"), "Artist", "{name}: untouched tags stay");
            assert!(back.cover.is_some(), "{name}: Keep keeps the cover");
            // remove the cover
            let back = write(&p, &TagEdit { cover: CoverEdit::Remove, ..Default::default() }).unwrap();
            assert_eq!(back.cover, None, "{name}: cover removed");
            assert_eq!(back.get("artist"), "Artist", "{name}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cover_from_a_video_frame() {
        if !have_ffmpeg() {
            return;
        }
        let d = tmpdir("frame");
        let v = make(&d, "v.mp4");
        let a = make(&d, "s.m4a");
        let jpg = d.join("frame.jpg");
        grab_frame(&v, 0.5, &jpg).unwrap();
        let back = write(&a, &TagEdit { cover: CoverEdit::Set(jpg), ..Default::default() }).unwrap();
        assert_eq!(back.cover_codec, "mjpeg");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_write_leaves_the_original_alone() {
        if !have_ffmpeg() {
            return;
        }
        let d = tmpdir("fail");
        let p = make(&d, "a.wav");
        let before = std::fs::read(&p).unwrap();
        let pic = picture(&d);
        let err = write(&p, &TagEdit { cover: CoverEdit::Set(pic), ..Default::default() }).unwrap_err();
        assert!(err.contains("cover"), "{err}");
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert!(!bak_path(&p).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_args_map_old_covers_out_and_mark_the_new_one() {
        let cur = MediaTags { video_streams: 1, audio_streams: 1, ..Default::default() };
        let mut e = TagEdit { cover: CoverEdit::Set("x.png".into()), ..Default::default() };
        e.set.insert("title".into(), "T".into());
        let a = write_args(Container::Mp4, &cur, &e, true).unwrap().join(" ");
        assert!(a.contains("-map 0:V? -map 0:a? -map 0:s? -map 1:v:0"), "{a}");
        assert!(a.contains("-disposition:v:1 attached_pic"), "{a}");
        assert!(a.contains("-metadata title=T"), "{a}");
        e.set.clear();
        e.set.insert("rating".into(), "5".into());
        assert!(write_args(Container::Mp4, &cur, &e, true).is_err(), "mp4 has no rating");
        e.set.insert("rating".into(), String::new());
        let a = write_args(Container::Mp4, &cur, &TagEdit { set: e.set.clone(), cover: CoverEdit::Keep }, false).unwrap().join(" ");
        assert!(a.starts_with("-map 0 -c copy"), "Keep maps everything: {a}");
        assert!(!a.contains("rating"), "clearing a tag the container can't hold is a no-op");
        assert!(write_args(Container::Wav, &cur, &TagEdit { cover: CoverEdit::Set("x".into()), ..Default::default() }, true).is_err());
    }

    #[test]
    fn mixed_values_across_files() {
        let mut a = MediaTags::default();
        a.values.insert("artist".into(), "X".into());
        a.values.insert("album".into(), "One".into());
        let mut b = a.clone();
        b.values.insert("album".into(), "Two".into());
        let m = merged(&[&a, &b]);
        assert_eq!(m["artist"], Some("X".into()));
        assert_eq!(m["album"], None, "mixed");
        assert_eq!(m["title"], Some(String::new()), "both empty");
    }

    #[test]
    fn probe_keys_are_normalised() {
        let v: Value = serde_json::json!({
            "format": {"duration": "3.5", "tags": {"TITLE": "t", "ALBUMARTIST": "aa", "TRACKNUMBER": "2", "lyrics-eng": "l"}},
            "streams": [
                {"index": 0, "codec_type": "audio"},
                {"index": 1, "codec_type": "video", "codec_name": "mjpeg", "disposition": {"attached_pic": 1}}
            ]
        });
        let t = parse_probe(&v);
        assert_eq!((t.get("title"), t.get("album_artist"), t.get("track"), t.get("lyrics")), ("t", "aa", "2", "l"));
        assert_eq!((t.cover, t.video_streams, t.audio_streams, t.duration), (Some(1), 0, 1, 3.5));
    }
}
