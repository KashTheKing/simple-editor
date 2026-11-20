//! ---- feat-retro-2000s-pack ----
//! The Classic 2000s pack in the app: `slideshow.make` (Make Slideshow: `engine::slideshow`), the
//! `slideshow.transitions` list, and the Gallery ▸ Titles ▸ Classic 2000s "Make Slideshow" card, which
//! runs the tool on what is selected as ONE undo step.

use super::tools_args::Args;
use super::*;
use crate::engine::{retro, slideshow};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

/// What a slideshow is made of when the caller names nothing: the Library's selection (in its order),
/// else the media under the timeline selection (in timeline order), else the Library's current item.
pub(super) fn picked_media(app: &App) -> Vec<Id> {
    let pictures = |id: &Id| app.project.asset(*id).is_some_and(|a| matches!(a.kind, ClipKind::Image | ClipKind::Video));
    if !app.library.sel_ids.is_empty() {
        return app.library.sel_ids.iter().copied().filter(pictures).collect();
    }
    let mut clips: Vec<&Clip> = app.selection.iter().filter_map(|&id| app.project.clip(id)).collect();
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<Id> = Vec::new();
    for c in clips {
        if c.asset != 0 && pictures(&c.asset) && !out.contains(&c.asset) {
            out.push(c.asset);
        }
    }
    if out.is_empty() {
        out.extend(app.library.selected.filter(pictures));
    }
    out
}

/// The tool's arguments as `slideshow::Opts` (defaults for what is missing).
fn opts(app: &App, args: &Value) -> slideshow::Opts {
    let a = Args(args);
    let d = slideshow::Opts::default();
    let text = |k: &str, def: Option<String>| match a.str(k) {
        Some("") => None,
        Some(s) => Some(s.to_string()),
        None => def,
    };
    slideshow::Opts {
        items: a.ids("asset_ids").unwrap_or_else(|| picked_media(app)),
        photo_secs: a.f64("photo_secs").unwrap_or(d.photo_secs),
        transition: a.str("transition").unwrap_or("random").to_string(),
        transition_secs: a.f64("transition_secs").unwrap_or(d.transition_secs),
        title: text("title", d.title),
        title_secs: a.f64("title_secs").unwrap_or(d.title_secs),
        credits: text("credits", d.credits),
        credits_secs: a.f64("credits_secs").unwrap_or(d.credits_secs),
        music: a.id("music"),
        fit_to_music: a.bool("fit_to_music").unwrap_or(false),
        ken_burns: a.bool("ken_burns").unwrap_or(true),
        at: a.t_or_playhead("at", app),
        seed: a.id("seed").unwrap_or_else(|| (app.project.next_id as u64).wrapping_mul(2654435761)),
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "slideshow.make",
        desc: "Make Slideshow (Classic 2000s): the photos and clips in order on one video track, a slow pan-and-zoom on each photo, a classic transition on every cut, a royal-blue title card first and a scrolling credits card last, optional background music - all as ordinary clips, blocks and transitions, ONE undo step. asset_ids: the media in order (default: the Library selection, else the timeline selection's media). photo_secs (default 5), transition: 'random' (a different classic one each cut; default), 'none', or an id from slideshow.transitions; transition_secs (default 1). title / credits: the cards' text ('' = no card; credits default 'Directed by / Starring / ...'), title_secs (4), credits_secs (8). music: an asset id with sound; fit_to_music: true stretches the photos so the show ends with the song (fits: false in the result when the song is too short). ken_burns (default true), at (default playhead), seed (same seed = same random transitions). Returns {clip_ids, transitions, duration, photo_secs, fits}.",
        args: &[
            "asset_ids:array:false:media asset ids in order (default: the selection)",
            "photo_secs:number:false:seconds per photo (default 5)",
            "transition:string:false:random (default), none, or a slideshow.transitions id",
            "transition_secs:number:false:default 1",
            "title:string:false:title card text ('' = none)",
            "title_secs:number:false:default 4",
            "credits:string:false:credits text ('' = none)",
            "credits_secs:number:false:default 8",
            "music:number:false:asset id of the background music",
            "fit_to_music:boolean:false:photo length fitted so it ends with the music",
            "ken_burns:boolean:false:pan and zoom on photos (default true)",
            "at:number:false:timeline seconds (default playhead)",
            "seed:number:false:random seed for the transitions",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let o = opts(app, args);
            let r = slideshow::make(&mut app.project, &o)?;
            app.selection = r.clips.clone();
            Ok(ToolOutcome::Done(json!({
                "clip_ids": r.clips,
                "transitions": r.transitions,
                "duration": r.duration,
                "photo_secs": r.photo_secs,
                "fits": r.fits,
            })))
        },
    },
    ToolDef {
        name: "slideshow.transitions",
        desc: "The Classic 2000s transitions slideshow.make can use: [{id, name, desc}]. They are also library transitions (Gallery ▸ Transitions ▸ Classic 2000s).",
        args: &[],
        kind: ToolKind::Read,
        run: |_app, _args| {
            let list: Vec<Value> = retro::TRANSITIONS
                .iter()
                .map(|(id, src)| {
                    let m = crate::engine::shaderlib::parse_meta(src);
                    json!({"id": id, "name": m.name, "desc": m.desc})
                })
                .collect();
            Ok(ToolOutcome::Done(json!(list)))
        },
    },
];

/// The Gallery's "Make Slideshow" card: the tool on the selection, one undo step, a toast either way.
pub(super) fn make_from_gallery(app: &mut App) {
    match app.run_tool_undoable("slideshow.make", &json!({})) {
        Ok(v) => {
            let secs = v["duration"].as_f64().unwrap_or(0.0);
            let n = v["transitions"].as_array().map_or(0, |a| a.len());
            app.toast(format!("Made a {secs:.0} s slideshow with {n} classic transitions"));
        }
        Err(e) => app.toast(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slideshow_tools_are_registered_and_the_make_tool_mutates() {
        let make = crate::mcp::tools::find("slideshow.make").expect("registered");
        assert!(matches!(make.kind, ToolKind::Mutate), "one undo step through run_tool_undoable");
        assert!(crate::mcp::tools::find("slideshow.transitions").is_some());
    }
}
