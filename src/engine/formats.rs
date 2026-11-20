//! ---- ws:parity-formats ----
//! File formats the importer and exporter gained for Shotcut parity (`plans/shotcut-comparison.md`):
//!
//! * **Soft subtitles**: the project's cues muxed into a finished MP4 / MOV / MKV / WebM as a subtitle
//!   track the viewer can switch on (`embed_subtitles`), next to the burn-in the export already had.
//!   A remux after the export (stream copy, seconds), so the export pipeline itself is untouched.
//! * **Subtitle extraction**: the text subtitle streams of a media file (`player::tracks` lists and
//!   pulls them; this module only names the containers and the codec that fits).
//! * **SVG**: an `.svg` becomes a PNG drawn by the HTML layer renderer (`engine::html`, Edge or the
//!   fast helper) - no new dependency, and gradients / text / filters draw as a browser draws them.
//!   `svg_page` wraps the file in a transparent page sized to the drawing.
//! * **AVIF**: decoded by ffmpeg like the other pictures (`media::is_image_path`); an animated AVIF
//!   is a video, as an animated GIF is.

use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------------
// soft subtitles

/// The subtitle codec a container carries text subtitles in, by output extension: MP4 / MOV keep
/// `mov_text` (3GPP timed text, what QuickTime, phones and browsers read), Matroska keeps SubRip and
/// WebM only WebVTT. None = the container has no text subtitle track (GIF, audio files, image runs).
pub fn soft_sub_codec(ext: &str) -> Option<&'static str> {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "mp4" | "m4v" | "mov" => Some("mov_text"),
        "mkv" => Some("srt"),
        "webm" => Some("webvtt"),
        _ => None,
    }
}

/// An ISO 639-2 language code for a subtitle track's tag: the code itself when it already is one
/// (three letters), a few common two-letter / English names mapped, "und" otherwise.
pub fn language_tag(lang: &str) -> String {
    let l = lang.trim().to_ascii_lowercase();
    let mapped = match l.as_str() {
        "en" | "english" => "eng",
        "es" | "spanish" => "spa",
        "fr" | "french" => "fre",
        "de" | "german" => "ger",
        "it" | "italian" => "ita",
        "pt" | "portuguese" => "por",
        "ja" | "japanese" => "jpn",
        "ko" | "korean" => "kor",
        "zh" | "chinese" => "chi",
        "ru" | "russian" => "rus",
        "ar" | "arabic" => "ara",
        "hi" | "hindi" => "hin",
        "nl" | "dutch" => "dut",
        "pl" | "polish" => "pol",
        "tr" | "turkish" => "tur",
        _ if l.len() == 3 && l.bytes().all(|b| b.is_ascii_lowercase()) => return l,
        _ => "und",
    };
    mapped.to_string()
}

/// ffmpeg arguments (after the executable) that copy `video`'s picture and sound unchanged and add
/// `subs` (an .srt) as one subtitle track named `title` in `lang`, writing `out`. Any subtitle track
/// `video` already had is dropped (the new one replaces it), so embedding twice never stacks tracks.
/// The track is not forced on: players offer it in their subtitle menu.
pub fn mux_args(video: &Path, subs: &Path, out: &Path, codec: &str, lang: &str, title: &str) -> Vec<String> {
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let mut a: Vec<String> = ["-y", "-v", "error", "-i"].map(String::from).to_vec();
    a.push(s(video));
    a.extend(["-f", "srt", "-i"].map(String::from));
    a.push(s(subs));
    a.extend(
        [
            "-map", "0:v?", "-map", "0:a?", "-map", "1:0", "-c:v", "copy", "-c:a", "copy", "-c:s", codec,
            "-map_metadata", "0", "-map_chapters", "0",
        ]
        .map(String::from),
    );
    a.push("-metadata:s:s:0".into());
    a.push(format!("language={}", language_tag(lang)));
    a.push("-metadata:s:s:0".into());
    a.push(format!("title={title}"));
    // mp4/mov name a track by its handler; mkv keeps `title`
    a.push("-metadata:s:s:0".into());
    a.push(format!("handler_name={title}"));
    a.extend(["-disposition:s:0", "0"].map(String::from));
    if codec == "mov_text" {
        a.extend(["-movflags", "+faststart"].map(String::from));
    }
    a.push(s(out));
    a
}

/// The cues of an export of timeline range `range` ([a, b) seconds; None = all), in the exported
/// file's own time: shifted by `a`, cut to the range, empty ones dropped.
pub fn cues_in_range(cues: &[(f64, f64, String)], range: Option<(f64, f64)>) -> Vec<(f64, f64, String)> {
    let (a, b) = range.unwrap_or((0.0, f64::INFINITY));
    cues.iter()
        .filter(|(s, e, t)| *e > a && *s < b && e > s && !t.trim().is_empty())
        .map(|(s, e, t)| (s.max(a) - a, e.min(b) - a, t.clone()))
        .collect()
}

/// Where the muxed file is written before it replaces (or sits beside) the original: same folder,
/// so the final rename never crosses drives.
pub fn mux_temp(video: &Path) -> PathBuf {
    let stem = video.file_stem().unwrap_or_default().to_string_lossy();
    let ext = video.extension().unwrap_or_default().to_string_lossy();
    video.with_file_name(format!("{stem}.subs-tmp.{ext}"))
}

/// `<stem> (subtitled).<ext>` beside `video`, numbered past any file already there: Embed in Video
/// File… never writes over the file it was given.
pub fn subtitled_copy(video: &Path) -> PathBuf {
    let stem = video.file_stem().unwrap_or_default().to_string_lossy();
    let ext = video.extension().unwrap_or_default().to_string_lossy();
    let mut out = video.with_file_name(format!("{stem} (subtitled).{ext}"));
    let mut n = 2;
    while out.exists() {
        out = video.with_file_name(format!("{stem} (subtitled {n}).{ext}"));
        n += 1;
    }
    out
}

/// Mux `srt` (SubRip text, `engine::subtitles::to_srt`) into `video` as a soft subtitle track.
/// `out` = None replaces `video` itself (an export that just finished); Some writes that file and
/// leaves `video` alone. Blocking (a stream copy: a few seconds for a long file); run it on a job
/// thread (`export::spawn_job`). A partial file is removed on failure.
pub fn embed_subtitles(video: &Path, srt: &str, out: Option<&Path>, lang: &str) -> Result<PathBuf, String> {
    let ext = video.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let codec = soft_sub_codec(&ext).ok_or_else(|| format!(".{ext} files have no subtitle track (use MP4, MOV, MKV or WebM)"))?;
    if srt.trim().is_empty() {
        return Err("there are no subtitles to embed".into());
    }
    let exe = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let tmp = mux_temp(video);
    let subs = tmp.with_extension("srt");
    std::fs::write(&subs, srt).map_err(|e| format!("{}: {e}", subs.display()))?;
    let r = crate::media::ffpipe::command(&exe).args(mux_args(video, &subs, &tmp, codec, lang, "Subtitles")).output();
    let _ = std::fs::remove_file(&subs);
    let fail = |e: String| {
        let _ = std::fs::remove_file(&tmp);
        Err(e)
    };
    match r {
        Err(e) => return fail(format!("ffmpeg: {e}")),
        Ok(o) if !o.status.success() => {
            let err = String::from_utf8_lossy(&o.stderr);
            return fail(err.lines().last().unwrap_or("ffmpeg failed").trim().to_string());
        }
        Ok(_) => {}
    }
    let dst = out.map(Path::to_path_buf).unwrap_or_else(|| video.to_path_buf());
    // std::fs::rename replaces an existing file on Windows (MOVEFILE_REPLACE_EXISTING)
    match std::fs::rename(&tmp, &dst) {
        Ok(()) => Ok(dst),
        Err(e) => fail(format!("{}: {e}", dst.display())),
    }
}

// ---------------------------------------------------------------------------------------------------
// SVG

/// The largest side an SVG is drawn at (its own size, scaled down to fit): a 4K frame's.
const SVG_MAX: (f64, f64) = (3840.0, 2160.0);

/// One length attribute (`width="120"`, `"2.5in"`, `"50mm"`, `"100%"`) in CSS pixels; a percentage
/// or an unknown unit is None (the viewBox decides then).
fn svg_length(v: &str) -> Option<f64> {
    let v = v.trim();
    let split = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e' || c == 'E')).unwrap_or(v.len());
    // "1em" would split at 'e': a unit starting with e is em / ex
    let (num, unit) = if v[..split].ends_with(['e', 'E']) && split > 0 { (&v[..split - 1], &v[split - 1..]) } else { (&v[..split], &v[split..]) };
    let n: f64 = num.parse().ok()?;
    let px = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "px" => n,
        "pt" => n * 96.0 / 72.0,
        "pc" => n * 16.0,
        "in" => n * 96.0,
        "cm" => n * 96.0 / 2.54,
        "mm" => n * 96.0 / 25.4,
        "em" => n * 16.0,
        "ex" => n * 8.0,
        _ => return None,
    };
    (px.is_finite() && px > 0.0).then_some(px)
}

/// The value of attribute `name` in the tag text `tag` (`<svg ...>`), quoted either way.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let low = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = low[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        // a whole attribute name: preceded by whitespace, followed by optional spaces and '='
        if !low[..at].ends_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let q = rest.chars().next()?;
        if q == '"' || q == '\'' {
            return rest[1..].find(q).map(|e| &rest[1..1 + e]);
        }
        return Some(rest.split(|c: char| c.is_ascii_whitespace() || c == '>').next().unwrap_or(""));
    }
    None
}

/// The root `<svg ...>` start tag of an SVG document (after any XML declaration, comments, doctype).
fn root_tag(svg: &str) -> Option<&str> {
    let low = svg.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let i = from + low[from..].find("<svg")?;
        let after = low.as_bytes().get(i + 4).copied();
        if after.is_some_and(|c| c.is_ascii_whitespace() || c == b'>' || c == b'/') {
            let end = i + low[i..].find('>')?;
            return Some(&svg[i..=end]);
        }
        from = i + 4;
    }
}

/// The natural size of an SVG in CSS pixels: its width / height attributes, else its viewBox (with
/// the one known side keeping the viewBox's aspect), else the browser default 300 x 150.
pub fn svg_size(svg: &str) -> (f64, f64) {
    let Some(tag) = root_tag(svg) else { return (300.0, 150.0) };
    let (w, h) = (attr(tag, "width").and_then(svg_length), attr(tag, "height").and_then(svg_length));
    let vb = attr(tag, "viewbox").and_then(|v| {
        let n: Vec<f64> = v.split([' ', ',', '\t', '\n']).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect();
        (n.len() == 4 && n[2] > 0.0 && n[3] > 0.0).then(|| (n[2], n[3]))
    });
    match (w, h, vb) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((vw, vh))) => (w, w * vh / vw),
        (None, Some(h), Some((vw, vh))) => (h * vw / vh, h),
        (None, None, Some(v)) => v,
        (Some(w), None, None) => (w, 150.0),
        (None, Some(h), None) => (300.0, h),
        (None, None, None) => (300.0, 150.0),
    }
}

/// The size an SVG of natural size `size` is drawn at: whole even pixels (the encoder and the
/// template format want even sides), at least 2, and scaled down to fit a 4K frame.
pub fn svg_draw_size(size: (f64, f64)) -> (u32, u32) {
    let fit = (SVG_MAX.0 / size.0).min(SVG_MAX.1 / size.1).min(1.0);
    let even = |v: f64| ((((v * fit).round() as u32) + 1) & !1).max(2);
    (even(size.0), even(size.1))
}

/// An HTML layer page (the `library/README.md` template format) that draws `svg` over a transparent
/// background at `w` x `h` CSS pixels. The XML prolog and doctype go (they are not HTML); the root
/// element is stretched to the page with its aspect kept (`preserveAspectRatio` stays the file's).
/// Scripts inside the SVG are removed by the renderer's own sanitiser like any page's.
pub fn svg_page(svg: &str, name: &str, (w, h): (u32, u32)) -> String {
    let body = strip_prolog(svg);
    let title: String = name.chars().filter(|c| !matches!(c, '<' | '>' | '&')).collect();
    format!(
        "<!doctype html>\n<html><head>\n<title>{title}</title>\n<meta name=\"se:kind\" content=\"layer\">\n\
         <meta name=\"se:size\" content=\"{w}x{h}\">\n<meta name=\"se:duration\" content=\"5s\">\n<style>\n\
         html, body {{ margin: 0; background: transparent; overflow: hidden; width: {w}px; height: {h}px; }}\n\
         body > svg {{ display: block; width: {w}px; height: {h}px; }}\n</style>\n</head><body>\n{body}\n</body></html>\n"
    )
}

/// `svg` without its `<?xml ...?>` declaration and `<!DOCTYPE ...>` (comments before the root stay).
fn strip_prolog(svg: &str) -> &str {
    let mut s = svg.trim_start_matches('\u{feff}').trim_start();
    loop {
        let low = s.get(..9).unwrap_or(s).to_ascii_lowercase();
        if low.starts_with("<?xml") || low.starts_with("<!doctype") {
            match s.find('>') {
                Some(e) => s = s[e + 1..].trim_start(),
                None => return "",
            }
        } else {
            return s;
        }
    }
}

/// Where a converted picture of `src` is written: `dir/<stem>.png`, numbered past files already there.
pub fn converted_path(dir: &Path, src: &Path, ext: &str) -> PathBuf {
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    let stem = if stem.is_empty() { "image".into() } else { stem };
    let mut out = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while out.exists() {
        out = dir.join(format!("{stem} {n}.{ext}"));
        n += 1;
    }
    out
}

/// Draw `svg_file` to a PNG at `out` (its natural size, fitted inside 4K), through the HTML layer
/// renderer. Blocking (a cold Edge takes about a second): a job thread only.
pub fn svg_to_png(svg_file: &Path, out: &Path) -> Result<(u32, u32), String> {
    let text = crate::engine::subtitles::read_file(svg_file)?;
    if root_tag(&text).is_none() {
        return Err(format!("{} is not an SVG drawing", svg_file.display()));
    }
    let size = svg_draw_size(svg_size(&text));
    let name = svg_file.file_stem().unwrap_or_default().to_string_lossy();
    let page = svg_page(&text, &name, size);
    // the page's own folder: an <image href="logo.png"> next to the SVG still loads
    let base = svg_file.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    // Edge, not the fast helper: a browser draws every SVG feature (filters, masks, text on a path)
    let view = crate::engine::html::View { base: &base, renderer: crate::model::HtmlRenderer::Edge, ..Default::default() };
    let png = crate::engine::html::render_png_in(&page, &[], 1.0, 0.0, &view)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(out, &png).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(crate::engine::html::png_size(&png).unwrap_or(size))
}

pub fn is_svg(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_subtitle_codecs_follow_the_container() {
        assert_eq!(soft_sub_codec("mp4"), Some("mov_text"));
        assert_eq!(soft_sub_codec(".MOV"), Some("mov_text"));
        assert_eq!(soft_sub_codec("mkv"), Some("srt"));
        assert_eq!(soft_sub_codec("webm"), Some("webvtt"));
        assert_eq!(soft_sub_codec("gif"), None);
        assert_eq!(soft_sub_codec("wav"), None);
        assert_eq!(language_tag("en"), "eng");
        assert_eq!(language_tag("Japanese"), "jpn");
        assert_eq!(language_tag("deu"), "deu");
        assert_eq!(language_tag(""), "und");
        assert_eq!(language_tag("auto"), "und");
    }

    #[test]
    fn mux_copies_streams_and_adds_one_text_track() {
        let a = mux_args(Path::new("C:/v/a.mp4"), Path::new("C:/v/a.srt"), Path::new("C:/v/out.mp4"), "mov_text", "en", "Subtitles");
        let j = a.join(" ");
        assert!(j.starts_with("-y -v error -i C:/v/a.mp4 -f srt -i C:/v/a.srt "), "{j}");
        assert!(j.contains("-map 0:v? -map 0:a? -map 1:0 -c:v copy -c:a copy -c:s mov_text"), "{j}");
        assert!(j.contains("language=eng") && j.contains("title=Subtitles") && j.contains("handler_name=Subtitles"), "{j}");
        assert!(j.contains("-disposition:s:0 0"), "offered, never forced on: {j}");
        assert!(j.contains("+faststart") && j.ends_with("C:/v/out.mp4"), "{j}");
        // an old subtitle track of the video is never mapped: only 0:v and 0:a come from it
        assert!(!j.contains("-map 0 ") && !j.contains("0:s"), "{j}");
        let m = mux_args(Path::new("a.mkv"), Path::new("a.srt"), Path::new("b.mkv"), "srt", "", "Subtitles").join(" ");
        assert!(m.contains("-c:s srt") && m.contains("language=und") && !m.contains("faststart"), "{m}");
        assert_eq!(mux_temp(Path::new("C:/v/My Film.mp4")), Path::new("C:/v/My Film.subs-tmp.mp4"));
    }

    #[test]
    fn export_ranges_shift_and_cut_the_cues() {
        let cues = vec![
            (0.0, 1.0, "before".to_string()),
            (1.5, 3.0, "straddles in".to_string()),
            (4.0, 5.0, "inside".to_string()),
            (5.5, 7.0, "straddles out".to_string()),
            (8.0, 9.0, "after".to_string()),
            (4.5, 4.8, "  ".to_string()),
        ];
        assert_eq!(cues_in_range(&cues, None).len(), 5, "all but the blank one");
        let r = cues_in_range(&cues, Some((2.0, 6.0)));
        assert_eq!(
            r,
            vec![(0.0, 1.0, "straddles in".to_string()), (2.0, 3.0, "inside".to_string()), (3.5, 4.0, "straddles out".to_string())]
        );
    }

    /// The real thing when ffmpeg is on the PATH: a tiny MP4 gets a mov_text track and an MKV an SRT
    /// one, and embedding again replaces the track instead of adding a second.
    #[test]
    fn embeds_a_soft_subtitle_track_with_real_ffmpeg() {
        let Some(exe) = crate::media::ffpipe::ffmpeg_exe() else { return };
        let Some(probe) = crate::media::ffpipe::ffprobe_exe() else { return };
        let dir = std::env::temp_dir().join(format!("se-embed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let srt = "1\n00:00:00,200 --> 00:00:00,800\nHello\n\n";
        for ext in ["mp4", "mkv"] {
            let v = dir.join(format!("v.{ext}"));
            let ok = crate::media::ffpipe::command(&exe)
                .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=red:s=64x64:d=1", "-f", "lavfi", "-i", "sine=d=1"])
                .args(["-c:v", "mpeg4", "-c:a", "aac", "-shortest"])
                .arg(&v)
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                continue; // an ffmpeg without these encoders: nothing to check here
            }
            let streams = |p: &Path| {
                let o = crate::media::ffpipe::command(&probe)
                    .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name", "-of", "csv=p=0"])
                    .arg(p)
                    .output()
                    .unwrap();
                String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect::<Vec<_>>()
            };
            let copy = subtitled_copy(&v);
            assert_eq!(embed_subtitles(&v, srt, Some(&copy), "en").unwrap(), copy);
            let want = if ext == "mp4" { "mov_text,subtitle" } else { "subrip,subtitle" };
            let s = streams(&copy);
            assert!(s.iter().any(|l| l == want), "{ext}: {s:?}");
            assert_eq!(s.len(), 3, "video + audio + one subtitle track: {s:?}");
            // in place, twice: still one subtitle track, no temp file left
            embed_subtitles(&copy, srt, None, "en").unwrap();
            assert_eq!(streams(&copy).iter().filter(|l| l.ends_with("subtitle")).count(), 1);
            assert!(!mux_temp(&copy).exists() && !mux_temp(&copy).with_extension("srt").exists());
        }
        assert!(embed_subtitles(&dir.join("x.gif"), srt, None, "").unwrap_err().contains("no subtitle track"));
        assert!(embed_subtitles(&dir.join("v.mp4"), " ", None, "").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subtitled_copies_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("se-formats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let v = dir.join("clip.mp4");
        assert_eq!(subtitled_copy(&v), dir.join("clip (subtitled).mp4"));
        std::fs::write(dir.join("clip (subtitled).mp4"), b"x").unwrap();
        assert_eq!(subtitled_copy(&v), dir.join("clip (subtitled 2).mp4"));
        assert_eq!(converted_path(&dir, Path::new("C:/art/logo.svg"), "png"), dir.join("logo.png"));
        std::fs::write(dir.join("logo.png"), b"x").unwrap();
        assert_eq!(converted_path(&dir, Path::new("C:/art/logo.svg"), "png"), dir.join("logo 2.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn svg_sizes_from_attributes_and_viewbox() {
        assert_eq!(svg_size(r#"<svg width="120" height="80"></svg>"#), (120.0, 80.0));
        assert_eq!(svg_size(r#"<?xml version="1.0"?><svg xmlns="x" viewBox="0 0 64 32"/>"#), (64.0, 32.0));
        assert_eq!(svg_size(r#"<svg width="200" viewBox="0,0,100,50">"#), (200.0, 100.0));
        assert_eq!(svg_size(r#"<svg height='1in' viewBox="0 0 2 1">"#), (192.0, 96.0));
        assert_eq!(svg_size(r#"<svg width="100%" height="100%" viewBox="0 0 30 10">"#), (30.0, 10.0));
        assert_eq!(svg_size(r#"<svg width="72pt" height="2em">"#), (96.0, 32.0));
        assert_eq!(svg_size("<svg>"), (300.0, 150.0));
        assert_eq!(svg_size("<html>not svg</html>"), (300.0, 150.0));
        // stroke-width / data-width are not width; <svgfoo> is not the root
        assert_eq!(svg_size(r#"<svgfoo width="1"/><svg stroke-width="9" data-height="7" width="40" height="20">"#), (40.0, 20.0));
    }

    #[test]
    fn svg_draw_sizes_are_even_and_fit_4k() {
        assert_eq!(svg_draw_size((121.0, 80.0)), (122, 80));
        assert_eq!(svg_draw_size((1.0, 0.4)), (2, 2));
        assert_eq!(svg_draw_size((10000.0, 1000.0)), (3840, 384));
        assert_eq!(svg_draw_size((1000.0, 9000.0)), (240, 2160));
    }

    #[test]
    fn svg_pages_are_transparent_layers_of_the_drawing() {
        let svg = "\u{feff}<?xml version=\"1.0\"?>\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x\">\n<svg width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\"/></svg>";
        let page = svg_page(svg, "Logo <1>", (10, 10));
        assert!(page.contains(r#"<meta name="se:size" content="10x10">"#), "{page}");
        assert!(page.contains("background: transparent"), "{page}");
        assert!(page.contains("<title>Logo 1</title>"), "{page}");
        assert!(!page.contains("<?xml") && !page.to_ascii_lowercase().contains("<!doctype svg"), "{page}");
        assert!(page.contains("<svg width=\"10\" height=\"10\"><rect"), "{page}");
        assert!(is_svg(Path::new("C:/a/B.SVG")) && !is_svg(Path::new("a.svgz")));
    }
}
