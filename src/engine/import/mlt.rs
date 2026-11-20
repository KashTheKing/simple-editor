//! ---- ws:parity-editing ----
//! Shotcut `.mlt` projects, for people switching over. Read with the same small XML reader and the same
//! honest per-item report as FCP7 XML / EDL (`super`).
//!
//! What maps: the profile (size, frame rate), every track of the main tractor in order (`shotcut:video`
//! / `shotcut:audio` playlists; the `background` black track is skipped), media clips and blanks, a
//! video clip's own sound as a linked clip on an audio track, `timewarp` speed, a muted / hidden track,
//! Shotcut's transitions (an entry pointing at a small tractor between two clips) as cross dissolves,
//! clip names (`shotcut:caption`), the Gain / Volume filter as clip volume, and the timeline markers
//! (`shotcut:markers`). What does not (other filters, text, colour clips, keyframes) is listed in the
//! report instead of failing the import.
use super::*;

/// MLT time: frames (`120`), a clock (`00:00:04.000`, `4.0`) or SMPTE (`00:00:04:00` / `;` drop frame).
pub(super) fn mlt_time(s: &str, fps: f64) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if !s.contains(':') && !s.contains(';') {
        // a bare number is frames (MLT's own unit); a decimal one is seconds
        return if s.contains('.') { s.parse().ok() } else { s.parse::<f64>().ok().map(|f| f / fps) };
    }
    let smpte = s.matches([':', ';']).count() == 3;
    let parts: Vec<f64> = s.split([':', ';']).map(|p| p.trim().parse::<f64>().ok()).collect::<Option<_>>()?;
    let t = match (&parts[..], smpte) {
        (&[h, m, sec, f], true) => h * 3600.0 + m * 60.0 + sec + f / fps,
        (&[h, m, sec], _) => h * 3600.0 + m * 60.0 + sec,
        (&[m, sec], _) => m * 60.0 + sec,
        _ => return None,
    };
    Some(t)
}

fn prop<'a>(el: &'a El, name: &str) -> &'a str {
    el.kids_named("property").find(|p| p.attr("name") == name).map(|p| p.text.trim()).unwrap_or("")
}

/// A producer / chain: what an entry plays.
struct Source<'a> {
    el: &'a El,
    service: &'a str,
    /// The file (a `timewarp` resource has its speed in front: `2.0:C:/a.mp4`).
    resource: String,
    speed: f64,
}

fn source_of(el: &El) -> Source<'_> {
    let service = prop(el, "mlt_service");
    let mut resource = prop(el, "resource").to_string();
    let mut speed = 1.0;
    if service == "timewarp" {
        let sp = prop(el, "warp_speed").parse::<f64>().ok();
        let res = prop(el, "warp_resource");
        match (sp, res.is_empty()) {
            (Some(sp), false) => {
                speed = sp;
                resource = res.to_string();
            }
            _ => {
                if let Some((a, b)) = resource.split_once(':') {
                    if let Ok(sp) = a.parse::<f64>() {
                        speed = sp;
                        resource = b.to_string();
                    }
                }
            }
        }
    }
    Source { el, service, resource, speed: if speed.abs() > 1e-6 { speed } else { 1.0 } }
}

/// The file behind a producer, as an asset (added once per producer id).
fn asset_for(
    src: &Source,
    root: &str,
    fps: f64,
    assets: &mut HashMap<String, Id>,
    subject: &str,
    b: &mut Build,
) -> Option<Id> {
    let key = src.resource.clone();
    if let Some(&id) = assets.get(&key) {
        return Some(id);
    }
    if !matches!(src.service, "avformat" | "avformat-novalidate" | "timewarp" | "qimage" | "pixbuf" | "glaxnimate")
        && !src.service.is_empty()
    {
        return None;
    }
    // Shotcut writes paths relative to `<mlt root="...">` when the media sits under the project folder
    let raw = if Path::new(&src.resource).is_relative() && !root.is_empty() {
        Path::new(root).join(&src.resource).to_string_lossy().into_owned()
    } else {
        src.resource.clone()
    };
    let path = b.resolve(&raw.replace('/', std::path::MAIN_SEPARATOR_STR), subject);
    if path.is_empty() {
        return None;
    }
    let len = mlt_time(prop(src.el, "length"), fps).unwrap_or(0.0);
    let image = is_image_path(&path) || matches!(src.service, "qimage" | "pixbuf");
    let has_audio = prop(src.el, "audio_index") != "-1" && !image;
    let w = prop(src.el, "meta.media.width").parse().unwrap_or(0);
    let h = prop(src.el, "meta.media.height").parse().unwrap_or(0);
    let asset: Asset = serde_json::from_value(serde_json::json!({
        "id": 0,
        "path": path,
        "kind": if image { "Image" } else { "Video" },
        "duration": len,
        "width": w,
        "height": h,
        "fps": fps,
        "audio_streams": if has_audio { serde_json::json!([{"index": 0, "channels": 2, "sample_rate": 48000}]) } else { serde_json::json!([]) },
    }))
    .ok()?;
    let id = b.add_asset(asset);
    assets.insert(key, id);
    Some(id)
}

struct Track<'a> {
    playlist: &'a El,
    audio: bool,
    hide_video: bool,
    hide_audio: bool,
}

pub(super) fn read_mlt(text: &str, b: &mut Build) -> Result<(), String> {
    let root = parse_xml(text)?;
    let mlt = if root.name == "mlt" { &root } else { root.kids.iter().find(|k| k.name == "mlt").ok_or("no <mlt> root")? };
    // profile
    let mut fps = 30.0;
    if let Some(p) = mlt.child("profile") {
        let (n, d) = (p.attr("frame_rate_num").parse::<f64>().unwrap_or(0.0), p.attr("frame_rate_den").parse::<f64>().unwrap_or(1.0));
        if n > 0.0 && d > 0.0 {
            fps = n / d;
        }
        let (w, h) = (p.attr("width").parse::<u32>().unwrap_or(0), p.attr("height").parse::<u32>().unwrap_or(0));
        if w > 0 && h > 0 {
            b.project.width = w;
            b.project.height = h;
        }
    }
    b.project.fps = crate::model::snap_fps(fps);
    let frame = 1.0 / fps;
    let mut by_id: HashMap<&str, &El> = HashMap::new();
    for k in &mlt.kids {
        let id = k.attr("id");
        if !id.is_empty() {
            by_id.insert(id, k);
        }
    }
    // the main tractor: Shotcut marks it (`shotcut` property) and it is the last one in the file
    let tractors: Vec<&El> = mlt.kids_named("tractor").collect();
    let main = tractors
        .iter()
        .rev()
        .find(|t| prop(t, "shotcut") == "1")
        .or(tractors.last())
        .copied()
        .ok_or("no <tractor> (timeline) in this .mlt file")?;
    let title = mlt.attr("title");
    if !title.is_empty() && !title.starts_with("Shotcut version") {
        b.project.name = title.to_string();
    }
    b.note(
        Level::Ok,
        format!("project '{}'", b.project.name),
        format!("{}×{} @ {fps:.3} fps", b.project.width, b.project.height),
    );

    let mut tracks: Vec<Track> = Vec::new();
    for t in main.kids_named("track") {
        let Some(pl) = by_id.get(t.attr("producer")).copied() else { continue };
        if pl.name != "playlist" || pl.attr("id") == "background" {
            continue;
        }
        let hide = t.attr("hide");
        let audio = prop(pl, "shotcut:audio") == "1" || (prop(pl, "shotcut:video") != "1" && hide == "video");
        tracks.push(Track {
            playlist: pl,
            audio,
            hide_video: hide == "video" || hide == "both",
            hide_audio: hide == "audio" || hide == "both",
        });
    }
    b.project.tracks.clear();
    let n_video = tracks.iter().filter(|t| !t.audio).count();
    let (mut vi, mut ai) = (0usize, 0usize);
    let mut assets: HashMap<String, Id> = HashMap::new();
    let mut transitions: Vec<(usize, f64, f64)> = Vec::new();
    for tr in &tracks {
        let kind = if tr.audio { TrackKind::Audio } else { TrackKind::Video };
        let ti = if tr.audio {
            // a video track's own sound sits on the first audio tracks; Shotcut's audio tracks after them
            let n = n_video + ai;
            ai += 1;
            track_slot(&mut b.project, kind, n)
        } else {
            let n = vi;
            vi += 1;
            track_slot(&mut b.project, kind, n)
        };
        let sound_ti = (!tr.audio && !tr.hide_audio).then(|| track_slot(&mut b.project, TrackKind::Audio, vi - 1));
        let name = prop(tr.playlist, "shotcut:name");
        if !name.is_empty() {
            b.project.tracks[ti].name = name.to_string();
        }
        let label = b.project.tracks[ti].name.clone();
        if (tr.audio && tr.hide_audio) || (!tr.audio && tr.hide_video) {
            b.project.tracks[ti].muted = true;
            b.note(Level::Ok, format!("track {label}"), "hidden in Shotcut: imported muted");
        }
        let mut t = 0.0;
        for e in &tr.playlist.kids {
            match e.name.as_str() {
                "blank" => t += mlt_time(e.attr("length"), fps).unwrap_or(0.0),
                "entry" => {
                    let fin = mlt_time(e.attr("in"), fps).unwrap_or(0.0);
                    let fout = mlt_time(e.attr("out"), fps).unwrap_or(fin);
                    let len = (fout - fin + frame).max(frame); // `out` is the last frame, inclusive
                    let Some(pel) = by_id.get(e.attr("producer")).copied() else {
                        b.note(Level::Skipped, format!("entry on {label}"), "its producer is not in the file");
                        t += len;
                        continue;
                    };
                    if pel.name == "tractor" {
                        // a Shotcut transition between the clips either side
                        transitions.push((ti, t, len));
                        t += len;
                        continue;
                    }
                    let src = source_of(pel);
                    let cap = prop(pel, "shotcut:caption");
                    let name = if cap.is_empty() {
                        Path::new(&src.resource).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                    } else {
                        cap.to_string()
                    };
                    let subject = format!("clip '{name}'");
                    let Some(aid) = asset_for(&src, mlt.attr("root"), fps, &mut assets, &subject, b) else {
                        let what = match src.service {
                            "color" | "colour" => "a colour clip",
                            _ => "a generator",
                        };
                        b.note(Level::Skipped, subject, format!("{what} ({}) has no counterpart; left as a gap", src.service));
                        t += len;
                        continue;
                    };
                    let image = b.project.asset(aid).is_some_and(|a| a.kind == ClipKind::Image);
                    let has_sound = b.project.asset(aid).is_some_and(|a| !a.audio_streams.is_empty());
                    let kinds: Vec<(ClipKind, usize)> = if tr.audio {
                        vec![(ClipKind::Audio, ti)]
                    } else if image {
                        vec![(ClipKind::Image, ti)]
                    } else {
                        let mut v = vec![(ClipKind::Video, ti)];
                        if let (true, Some(sti)) = (has_sound, sound_ti) {
                            v.push((ClipKind::Audio, sti));
                        }
                        v
                    };
                    let link = if kinds.len() > 1 { b.project.new_id() } else { 0 };
                    let gain = volume_of(pel, e, &subject, b);
                    for (k, slot) in kinds {
                        let id = b.project.new_id();
                        let mut c = Clip::new(id, k, name.clone(), t, len.max(MIN_CLIP));
                        c.asset = aid;
                        c.link = link;
                        c.src_in = if image { 0.0 } else { fin * src.speed.abs() };
                        if src.speed < 0.0 {
                            c.reverse = true;
                        }
                        c.speed = src.speed.abs();
                        if let Some(g) = gain {
                            c.volume.set_at(0.0, g);
                        }
                        place(&mut b.project, slot, c);
                    }
                    if (src.speed - 1.0).abs() > 1e-6 {
                        b.note(Level::Ok, subject.clone(), format!("speed {:.0} %", src.speed * 100.0));
                    }
                    t += len;
                }
                _ => {}
            }
        }
        for f in tr.playlist.kids_named("filter") {
            b.note(Level::Warning, format!("track {label}"), format!("track filter '{}' not imported", filter_name(f)));
        }
    }
    if b.project.tracks.is_empty() {
        b.project.add_track(TrackKind::Video);
        b.project.add_track(TrackKind::Audio);
    }
    // Shotcut transition: A | T(len) | B. Ours sits on the cut, so A and B each reach half into it.
    for (ti, at, len) in transitions {
        let half = len / 2.0;
        let cut = at + half;
        let clips = &b.project.tracks[ti].clips;
        let left = clips.iter().find(|c| (c.end() - at).abs() < frame / 2.0).map(|c| (c.id, c.link));
        let right = clips.iter().find(|c| (c.start - (at + len)).abs() < frame / 2.0).map(|c| (c.id, c.link));
        // the clip and its linked sound move together
        let ids_of = |p: &Project, (id, link): (Id, Id)| -> Vec<Id> {
            p.all_clips().filter(|(_, c)| c.id == id || (link != 0 && c.link == link)).map(|(_, c)| c.id).collect()
        };
        for id in left.map(|l| ids_of(&b.project, l)).unwrap_or_default() {
            if let Some(c) = b.project.clip_mut(id) {
                c.duration += half;
            }
        }
        for id in right.map(|r| ids_of(&b.project, r)).unwrap_or_default() {
            if let Some(c) = b.project.clip_mut(id) {
                c.start -= half;
                c.duration += half;
                c.src_in = (c.src_in - half * c.speed).max(0.0);
            }
        }
        let right = right.map(|r| r.0);
        match right.and_then(|id| b.project.add_transition(id, TransitionKind::CrossFade, len)) {
            Some(_) => b.note(Level::Ok, "transition", format!("cross dissolve of {len:.2} s at {cut:.2} s")),
            None => b.note(Level::Warning, "transition", format!("at {at:.2} s dropped: not between two clips")),
        }
    }
    link_stacks(&mut b.project);
    // timeline markers
    if let Some(list) = main.kids_named("properties").find(|p| p.attr("name") == "shotcut:markers") {
        for m in list.kids_named("properties") {
            let text = prop(m, "text").to_string();
            let Some(start) = mlt_time(prop(m, "start"), fps) else { continue };
            let end = mlt_time(prop(m, "end"), fps).unwrap_or(start);
            let id = b.project.add_marker(start, text.clone());
            if let Some(mm) = b.project.marker_mut(id) {
                mm.duration = (end - start).max(0.0);
            }
            b.note(Level::Ok, format!("marker '{text}'"), format!("at {start:.2} s"));
        }
    }
    for f in main.kids_named("filter") {
        b.note(Level::Warning, "timeline", format!("filter '{}' on the whole timeline not imported", filter_name(f)));
    }
    Ok(())
}

fn filter_name(f: &El) -> String {
    let n = prop(f, "shotcut:filter");
    if n.is_empty() { prop(f, "mlt_service").to_string() } else { n.to_string() }
}

/// A Gain / Volume filter on the clip (`volume` service, `level` in dB) as a linear gain; every other
/// filter is reported.
fn volume_of(producer: &El, entry: &El, subject: &str, b: &mut Build) -> Option<f64> {
    let mut gain = None;
    for f in producer.kids_named("filter").chain(entry.kids_named("filter")) {
        let service = prop(f, "mlt_service");
        if prop(f, "disable") == "1" || service == "audiolevel" {
            continue;
        }
        if service == "volume" {
            let level = prop(f, "level");
            match level.trim_end_matches("dB").trim().parse::<f64>() {
                Ok(db) if !level.contains('=') => {
                    gain = Some(10f64.powf(db / 20.0));
                    b.note(Level::Ok, subject, format!("gain {db:+.1} dB"));
                }
                _ => b.note(Level::Warning, subject, "keyframed gain not imported"),
            }
            continue;
        }
        b.note(Level::Warning, subject, format!("filter '{}' not imported", filter_name(f)));
    }
    gain
}

#[cfg(test)]
mod tests {
    use super::super::*;

    const MLT: &str = r#"<?xml version="1.0" standalone="no"?>
<mlt LC_NUMERIC="C" version="7.24.0" title="Shotcut version 24.02.29" producer="main_bin">
  <profile description="HD 1080p 25 fps" width="1920" height="1080" progressive="1" frame_rate_num="25" frame_rate_den="1" colorspace="709"/>
  <playlist id="main_bin"><property name="xml_retain">1</property></playlist>
  <producer id="black" in="00:00:00.000" out="00:00:20.000">
    <property name="mlt_service">color</property><property name="resource">0</property>
  </producer>
  <playlist id="background"><entry producer="black" in="00:00:00.000" out="00:00:20.000"/></playlist>
  <chain id="chain0" out="00:00:30.000">
    <property name="length">00:00:30.040</property>
    <property name="resource">C:/nowhere/beach.mp4</property>
    <property name="mlt_service">avformat-novalidate</property>
    <property name="shotcut:caption">Beach</property>
    <filter id="filter0"><property name="mlt_service">volume</property><property name="level">-6</property></filter>
    <filter id="filter1"><property name="mlt_service">frei0r.saturat0r</property><property name="shotcut:filter">saturation</property></filter>
  </chain>
  <chain id="chain1" out="00:00:30.000">
    <property name="length">00:00:30.040</property>
    <property name="resource">C:/nowhere/city.mp4</property>
    <property name="mlt_service">avformat-novalidate</property>
  </chain>
  <producer id="producer2" in="00:00:00.000" out="00:00:10.000">
    <property name="mlt_service">timewarp</property>
    <property name="resource">2:C:/nowhere/clouds.mp4</property>
    <property name="warp_speed">2</property>
    <property name="warp_resource">C:/nowhere/clouds.mp4</property>
    <property name="audio_index">-1</property>
  </producer>
  <producer id="pcolor" in="0" out="49"><property name="mlt_service">color</property><property name="resource">#ff0000</property></producer>
  <tractor id="tractor0" in="00:00:00.000" out="00:00:00.960">
    <property name="shotcut:transition">lumaMix</property>
    <track producer="chain0" in="00:00:04.000" out="00:00:04.960"/>
    <track producer="chain1" in="00:00:00.000" out="00:00:00.960"/>
  </tractor>
  <playlist id="playlist0">
    <property name="shotcut:video">1</property>
    <property name="shotcut:name">V1</property>
    <entry producer="chain0" in="00:00:01.000" out="00:00:03.960"/>
    <entry producer="tractor0" in="00:00:00.000" out="00:00:00.960"/>
    <entry producer="chain1" in="00:00:01.000" out="00:00:02.960"/>
    <blank length="00:00:01.000"/>
    <entry producer="pcolor" in="0" out="49"/>
  </playlist>
  <playlist id="playlist1">
    <property name="shotcut:video">1</property>
    <property name="shotcut:name">V2</property>
    <entry producer="producer2" in="00:00:02.000" out="00:00:03.960"/>
  </playlist>
  <tractor id="tractor1" title="Shotcut version 24.02.29" global_feed="1" in="00:00:00.000" out="00:00:10.000">
    <property name="shotcut">1</property>
    <properties name="shotcut:markers">
      <properties name="0"><property name="text">Chorus</property><property name="start">00:00:02.000</property><property name="end">00:00:02.000</property></properties>
    </properties>
    <track producer="background"/>
    <track producer="playlist0"/>
    <track producer="playlist1" hide="audio"/>
  </tractor>
</mlt>"#;

    #[test]
    fn mlt_time_reads_frames_clock_and_smpte() {
        let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-9);
        assert!(near(mlt::mlt_time("50", 25.0), 2.0), "frames");
        assert!(near(mlt::mlt_time("00:00:02.500", 25.0), 2.5), "clock");
        assert!(near(mlt::mlt_time("00:00:02:05", 25.0), 2.2), "smpte");
        assert!(near(mlt::mlt_time("00:00:02;05", 25.0), 2.2), "drop-frame separator");
        assert!(near(mlt::mlt_time("1.5", 25.0), 1.5), "decimal seconds");
        assert_eq!(mlt::mlt_time("junk", 25.0), None);
    }

    #[test]
    fn shotcut_project_imports_tracks_clips_transitions_and_markers() {
        let dir = std::env::temp_dir().join(format!("se-mlt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("my edit.mlt");
        std::fs::write(&path, MLT).unwrap();
        let r = import_file(&path).unwrap();
        let p = &r.project;
        assert_eq!((p.width, p.height), (1920, 1080));
        assert!((p.fps - 25.0).abs() < 1e-9);
        let v1 = &p.tracks[p.video_tracks()[0]];
        // A [0, 3) + T [3, 4) + B [4, 6): A and B each reach half into the transition
        let names: Vec<&str> = v1.clips.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Beach", "city.mp4"], "the colour clip is a gap: {:?}", r.to_markdown());
        let (a, b) = (&v1.clips[0], &v1.clips[1]);
        assert!((a.start - 0.0).abs() < 1e-6 && (a.end() - 3.5).abs() < 1e-6, "{} {}", a.start, a.end());
        assert!((b.start - 3.5).abs() < 1e-6 && (b.end() - 6.0).abs() < 1e-6, "{} {}", b.start, b.end());
        assert!((a.src_in - 1.0).abs() < 1e-6 && (b.src_in - 0.5).abs() < 1e-6);
        assert_eq!(v1.transitions.len(), 1, "the Shotcut transition became a dissolve");
        assert!((a.volume.at(0.0) - 10f64.powf(-6.0 / 20.0)).abs() < 1e-6, "-6 dB gain");
        // the V1 clips' sound is linked on an audio track; V2 hides its audio and the clip has none
        let audio: Vec<&Clip> = p.audio_tracks().iter().flat_map(|&i| p.tracks[i].clips.iter()).collect();
        assert_eq!(audio.len(), 2, "{:?}", audio.iter().map(|c| &c.name).collect::<Vec<_>>());
        assert!(audio.iter().all(|c| c.link != 0));
        // V2: a 2x timewarp clip from 2 s of warped time = 4 s of source
        let v2 = &p.tracks[p.video_tracks()[1]];
        assert_eq!(v2.clips.len(), 1);
        assert!((v2.clips[0].speed - 2.0).abs() < 1e-9 && (v2.clips[0].src_in - 4.0).abs() < 1e-6);
        assert_eq!(p.markers.len(), 1);
        assert_eq!(p.markers[0].name, "Chorus");
        let md = r.to_markdown();
        assert!(md.contains("saturation") && md.contains("not imported"), "unmapped filters are reported: {md}");
        assert!(r.missing_media > 0, "the files are not on this machine");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
