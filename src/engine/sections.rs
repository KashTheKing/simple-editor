//! ---- ws:zones-markers ----
//! Export between markers: the main timeline cut into sections at its markers, one export job each
//! (`ui::app::tools_zones` queues them on the render queue), named `<project> - <marker name>`.
//!
//! - The span is In/Out when set (either end alone too), else the whole timeline.
//! - Point markers inside the span split it: a section runs from one marker to the next and takes the
//!   name of the marker it starts at. What lies before the first marker is "Start".
//! - A range marker is a section of its own (its start to its end, clipped to the span).
//! - With only range markers, the sections are just those.
//!
//! Markers of nested sequences and of clips do not count: an export renders the main timeline.

use crate::model::Project;

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub name: String,
    pub from: f64,
    pub to: f64,
}

/// The sections of `p` (the main timeline, as exported), in time order.
pub fn sections(p: &Project) -> Vec<Section> {
    let from = p.in_point.unwrap_or(0.0).max(0.0);
    let to = p.out_point.unwrap_or_else(|| p.duration());
    let frame = 1.0 / p.fps.max(1.0);
    if to - from < frame {
        return Vec::new();
    }
    let mine: Vec<&crate::model::Marker> = p.markers.iter().filter(|m| m.sequence.is_none()).collect();
    let name = |m: &crate::model::Marker, n: usize| {
        let s = m.name.trim();
        if s.is_empty() {
            format!("Marker {n}")
        } else {
            s.to_string()
        }
    };
    let mut out = Vec::new();
    let mut points: Vec<&crate::model::Marker> =
        mine.iter().copied().filter(|m| m.duration <= 0.0 && m.t > from + frame * 0.5 && m.t < to - frame * 0.5).collect();
    points.sort_by(|a, b| a.t.total_cmp(&b.t));
    // a point marker exactly at the span's start names the first section instead of splitting it
    let at_start = mine.iter().copied().find(|m| m.duration <= 0.0 && (m.t - from).abs() <= frame * 0.5);
    if !points.is_empty() || at_start.is_some() {
        let mut bounds: Vec<(f64, String)> =
            vec![(from, at_start.map(|m| name(m, 1)).unwrap_or_else(|| "Start".to_string()))];
        for (i, m) in points.iter().enumerate() {
            bounds.push((m.t, name(m, i + 2)));
        }
        for (i, (a, n)) in bounds.iter().enumerate() {
            let b = bounds.get(i + 1).map(|x| x.0).unwrap_or(to);
            if b - a >= frame {
                out.push(Section { name: n.clone(), from: *a, to: b });
            }
        }
    }
    for (i, m) in mine.iter().filter(|m| m.duration > 0.0).enumerate() {
        let (a, b) = (m.t.max(from), (m.t + m.duration).min(to));
        if b - a >= frame {
            out.push(Section { name: name(m, i + 1), from: a, to: b });
        }
    }
    out.sort_by(|a, b| a.from.total_cmp(&b.from));
    out
}

/// `<project> - <section>.<ext>` with the characters Windows refuses in a file name replaced.
pub fn file_name(project: &str, section: &str, ext: &str) -> String {
    let clean = |s: &str| -> String {
        let t: String = s.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
        t.trim().trim_end_matches('.').to_string()
    };
    let p = clean(project);
    let p = if p.is_empty() { "Untitled".to_string() } else { p };
    let ext = if ext.is_empty() { "mp4" } else { ext };
    format!("{p} - {}.{ext}", clean(section))
}

/// One file name per section, a repeated name numbered (" 2", " 3", …) so no job overwrites another.
pub fn file_names(project: &str, secs: &[Section], ext: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for s in secs {
        let mut n = 1;
        let mut f = file_name(project, &s.name, ext);
        while seen.iter().any(|x| x.eq_ignore_ascii_case(&f)) {
            n += 1;
            f = file_name(project, &format!("{} {n}", s.name), ext);
        }
        seen.push(f.clone());
        out.push(f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};

    fn project(len: f64) -> Project {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        p.tracks[v1].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, len));
        p.next_id = 10;
        p
    }
    fn spans(s: &[Section]) -> Vec<(&str, f64, f64)> {
        s.iter().map(|x| (x.name.as_str(), x.from, x.to)).collect()
    }

    #[test]
    fn point_markers_split_the_timeline() {
        let mut p = project(60.0);
        p.add_marker(20.0, "Verse");
        p.add_marker(45.0, "Chorus");
        assert_eq!(spans(&sections(&p)), vec![("Start", 0.0, 20.0), ("Verse", 20.0, 45.0), ("Chorus", 45.0, 60.0)]);
        // a marker at 0 names the first section
        p.add_marker(0.0, "Intro");
        assert_eq!(sections(&p)[0].name, "Intro");
        assert_eq!(sections(&p).len(), 3);
    }

    #[test]
    fn in_out_bounds_the_sections() {
        let mut p = project(60.0);
        p.add_marker(10.0, "A");
        p.add_marker(30.0, "B");
        p.add_marker(50.0, "C");
        (p.in_point, p.out_point) = (Some(10.0), Some(40.0));
        assert_eq!(spans(&sections(&p)), vec![("A", 10.0, 30.0), ("B", 30.0, 40.0)], "C lies past Out");
        p.out_point = None;
        assert_eq!(sections(&p).last().map(|s| s.to), Some(60.0));
    }

    #[test]
    fn range_markers_are_sections_of_their_own() {
        let mut p = project(60.0);
        let r = p.add_marker(5.0, "Clip one");
        p.marker_mut(r).unwrap().duration = 10.0;
        let r = p.add_marker(50.0, "Tail");
        p.marker_mut(r).unwrap().duration = 30.0;
        assert_eq!(spans(&sections(&p)), vec![("Clip one", 5.0, 15.0), ("Tail", 50.0, 60.0)], "clipped to the end");
        assert!(sections(&project(60.0)).is_empty(), "no markers, no sections");
    }

    #[test]
    fn markers_of_a_sequence_do_not_count() {
        let mut p = project(60.0);
        let m = p.add_marker(20.0, "Elsewhere");
        p.marker_mut(m).unwrap().sequence = Some(99);
        assert!(sections(&p).is_empty());
    }

    #[test]
    fn names_are_project_dash_marker_and_never_collide() {
        assert_eq!(file_name("My Vlog", "Intro", "mp4"), "My Vlog - Intro.mp4");
        assert_eq!(file_name("a/b", "Q: why?", ""), "a_b - Q_ why_.mp4");
        assert_eq!(file_name("", "x", "mov"), "Untitled - x.mov");
        let secs = vec![
            Section { name: "Take".into(), from: 0.0, to: 1.0 },
            Section { name: "take".into(), from: 1.0, to: 2.0 },
            Section { name: "Take".into(), from: 2.0, to: 3.0 },
        ];
        assert_eq!(file_names("P", &secs, "mp4"), vec!["P - Take.mp4", "P - take 2.mp4", "P - Take 3.mp4"]);
    }
}
