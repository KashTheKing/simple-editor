//! ws:forum-ui - "What do you want to do?": plain phrases in the Ctrl+P box ("cut out a part",
//! "freeze", "split screen", "blur a face", "beep", "ticker", "rotate", "duck music") lead straight to
//! the one action that does it, with its shortcut beside it (plans/shotcut-forum-insights.md, problem 2:
//! the basic jobs people otherwise learn from 8-step forum recipes). One Enter runs it, so every phrase
//! is at most two clicks away (open the palette, pick the row).
//!
//! Pure data plus a scorer, like the rest of `ui::palette`: `palette_ctl::dispatch` runs a `Do`.

use crate::hotkeys::{Action, Hotkeys};
use crate::ui::gallery::GalleryTab;
use crate::ui::menu::MenuSnapshot;
use crate::ui::palette::{fuzzy_score, Command, Row};
use crate::ui::tools::Glyph;

/// What a task row runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Do {
    Act(Action),
    /// A tool with fixed JSON arguments, as the user's own undoable edit.
    Tool(&'static str, &'static str),
    /// Open the Gallery on this tab.
    Gallery(GalleryTab),
}

pub struct Task {
    /// The row's label: what it does, in the user's words.
    pub label: &'static str,
    /// Phrases people type for it (lower case). A query that starts one of these, or contains it, matches.
    pub words: &'static [&'static str],
    pub what: Do,
    /// The tooltip: what to have ready, what happens.
    pub hint: &'static str,
}

const fn t(label: &'static str, words: &'static [&'static str], what: Do, hint: &'static str) -> Task {
    Task { label, words, what, hint }
}

/// Every task, roughly in the order the forums ask for them.
pub const TASKS: &[Task] = &[
    t("Cut out a part", &["cut out", "cut a part", "cut part", "remove a section", "remove part", "delete a part", "delete section", "cut section", "cut the middle"], Do::Act(Action::RippleDeleteInOut),
        "Mark In (I) and Out (O) around the part first: it is removed and the gap closes"),
    t("Split at the playhead", &["split", "cut here", "cut at", "razor", "cut clip", "cut video"], Do::Act(Action::Split),
        "Cuts the clips under the playhead in two"),
    t("Delete and close the gap", &["close gap", "ripple delete", "delete and close", "remove gap", "no gap"], Do::Act(Action::RippleDelete),
        "Removes the selected clips and pulls what follows left"),
    t("Freeze a frame", &["freeze", "still frame", "hold frame", "pause frame", "freeze frame", "stop motion"], Do::Act(Action::FreezeFrame),
        "Holds the frame under the playhead"),
    t("Split screen side by side", &["split screen", "side by side", "two videos", "2 videos", "two up", "compare videos"], Do::Tool("clip.layout", r#"{"layout":"2up"}"#),
        "Select two clips stacked on different tracks first"),
    t("Grid of four videos", &["grid", "2x2", "four videos", "4 videos", "collage", "quad"], Do::Tool("clip.layout", r#"{"layout":"2x2"}"#),
        "Select up to four clips stacked on different tracks first"),
    t("Picture in picture", &["picture in picture", "pip", "corner video", "small video", "webcam corner", "facecam"], Do::Tool("clip.layout", r#"{"layout":"pip"}"#),
        "Select the full-frame clip, then the one for the corner"),
    t("Blur a face or text", &["blur", "censor", "hide face", "blur face", "blur text", "pixelate", "hide plate", "hide name"], Do::Tool("clip.censor", r#"{"what":"blur"}"#),
        "Over In to Out, else over the selected clips. Move the box in the Preview"),
    t("Bleep out a word", &["beep", "bleep", "mute a word", "censor word", "swear", "silence a word", "mute part"], Do::Tool("clip.censor", r#"{"what":"mute"}"#),
        "Mark In and Out around the word: the sound drops out there"),
    t("Scrolling news ticker", &["ticker", "scrolling text", "crawl", "news ticker", "moving text", "marquee"], Do::Tool("titles.place", r#"{"name":"News Ticker"}"#),
        "Places a bar with crawling text at the playhead; edit the text in the Inspector"),
    t("Rolling credits", &["credits", "end credits", "rolling credits", "scroll up"], Do::Tool("titles.place", r#"{"name":"Scrolling Credits"}"#),
        "Places credits that rise through the frame at the playhead"),
    t("Rotate 90°", &["rotate", "turn sideways", "sideways", "upside down", "portrait", "90 degrees"], Do::Tool("clip.rotate", r#"{"degrees":90}"#),
        "Turns the selected clips a quarter turn clockwise"),
    t("Duck the music under the voice", &["duck", "music too loud", "music louder", "voice clear", "make the voice clear", "lower music", "hear the voice"], Do::Act(Action::AutoDuck),
        "Select the voice and the music clips first"),
    t("Make it louder", &["louder", "too quiet", "normalize", "volume", "boost audio"], Do::Act(Action::Normalize),
        "Brings the selected clips to a standard loudness"),
    t("Speed up or slow down", &["speed", "slow motion", "slow down", "fast forward", "speed up", "timelapse", "reverse"], Do::Act(Action::Retime),
        "Speed, reverse and ramps for the selected clip"),
    t("Remove silences", &["silence", "pauses", "dead air", "remove silence", "auto cut", "jump cut"], Do::Act(Action::AutoCut),
        "Finds the quiet parts and cuts them out"),
    t("Add captions", &["captions", "subtitles", "transcribe", "speech to text", "auto captions"], Do::Act(Action::GetCaptions),
        "Transcribes the speech into captions"),
    t("Record a voiceover", &["voiceover", "voice over", "narrate", "record voice", "microphone", "record audio"], Do::Act(Action::Voiceover),
        "Records from the microphone onto a new track"),
    t("Add text", &["text", "title", "add title", "write on video", "words"], Do::Act(Action::AddText),
        "A text clip at the playhead"),
    t("Add a transition", &["transition", "fade", "dissolve", "crossfade", "fade in", "fade out"], Do::Act(Action::AddTransition),
        "On the selected cut; more in the Gallery"),
    t("Fix the colour", &["color", "colour", "too dark", "brighten", "auto color", "white balance"], Do::Act(Action::AutoColor),
        "One-click colour correction on the selected clips"),
    t("Replace the footage", &["replace", "replace footage", "replace media", "swap clip", "swap video"], Do::Act(Action::ReplaceWithLibrarySelection),
        "Swaps the selected clip for the media picked in the Library, keeping its cuts"),
    t("Find missing files", &["missing", "missing files", "relink", "offline", "media offline", "moved files"], Do::Act(Action::RelinkMedia),
        "Points offline clips at their files again"),
    t("Effects", &["effect", "effects", "filter", "filters", "look"], Do::Gallery(GalleryTab::Effects),
        "Browse effects; drag one onto a clip"),
    t("Export for YouTube or TikTok", &["export", "youtube", "tiktok", "instagram", "save video", "render", "upload"], Do::Act(Action::Workspace7),
        "The Export page, with a card per platform"),
];

/// How well `query` asks for `task`: a phrase it starts beats one it contains, beats a fuzzy label match.
fn score(task: &Task, q: &str) -> Option<u32> {
    let best = task
        .words
        .iter()
        .filter_map(|w| {
            if w.starts_with(q) || q.starts_with(w) {
                Some(200 + w.len().min(q.len()) as u32)
            } else if w.contains(q) && q.len() >= 3 {
                Some(120)
            } else {
                None
            }
        })
        .max();
    best.or_else(|| (q.len() >= 3).then(|| fuzzy_score(q, task.label)).flatten())
}

/// The task rows for a go-to `query` (best first, at most `MAX`), headed "Do". Empty for an empty query.
/// Shortcuts and disabled reasons come from `snap`, like every other palette row.
pub fn task_rows(snap: &MenuSnapshot, query: &str) -> Vec<Row> {
    const MAX: usize = 4;
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u32, usize)> = TASKS.iter().enumerate().filter_map(|(i, t)| Some((score(t, &q)?, i))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter()
        .take(MAX)
        .map(|(_, i)| {
            let task = &TASKS[i];
            let (shortcut, reason) = match task.what {
                Do::Act(a) => (snap.shortcuts.get(&a).map(Hotkeys::format).unwrap_or_default(), snap.disabled.get(&a).copied()),
                _ => (String::new(), None),
            };
            Row {
                cmd: Command::Task(i),
                label: task.label.to_string(),
                shortcut,
                glyph: Some(Glyph::Star),
                enabled: reason.is_none(),
                reason: reason.or(Some(task.hint)),
                group: "Do",
                ..Default::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> MenuSnapshot {
        MenuSnapshot::new(&Hotkeys::defaults(), &Default::default(), |_| Ok(()))
    }

    fn first(q: &str) -> &'static str {
        let rows = task_rows(&snap(), q);
        match rows.first().map(|r| &r.cmd) {
            Some(Command::Task(i)) => TASKS[*i].label,
            other => panic!("no task row for '{q}': {other:?}"),
        }
    }

    /// The plan's phrases each reach their action as the first row: one Enter after Ctrl+P.
    #[test]
    fn each_forum_phrase_reaches_its_action_first() {
        for (q, label) in [
            ("cut out a part", "Cut out a part"),
            ("freeze", "Freeze a frame"),
            ("split screen", "Split screen side by side"),
            ("blur", "Blur a face or text"),
            ("beep", "Bleep out a word"),
            ("ticker", "Scrolling news ticker"),
            ("rotate", "Rotate 90°"),
            ("duck music", "Duck the music under the voice"),
            ("make the voice clear", "Duck the music under the voice"),
            ("picture in picture", "Picture in picture"),
            ("missing files", "Find missing files"),
        ] {
            assert_eq!(first(q), label, "query '{q}'");
        }
    }

    #[test]
    fn action_tasks_show_their_shortcut() {
        let rows = task_rows(&snap(), "freeze");
        assert_eq!(rows[0].shortcut, Hotkeys::format(&Action::FreezeFrame.default_shortcut().unwrap()));
        assert_eq!(rows[0].group, "Do");
    }

    #[test]
    fn empty_query_and_nonsense_show_no_tasks() {
        assert!(task_rows(&snap(), "").is_empty());
        assert!(task_rows(&snap(), "qzxv").is_empty());
        assert!(task_rows(&snap(), "a").len() <= 4);
    }

    /// Plain-labels rule: a task label never carries an explanation after it.
    #[test]
    fn task_labels_have_no_tails() {
        for t in TASKS {
            assert!(!t.label.contains(" - ") && !t.label.contains(':') && !t.label.contains('('), "{}", t.label);
            assert!(!t.hint.is_empty());
        }
    }

    /// Every tool a task names is registered, with arguments that parse.
    #[test]
    fn task_tools_exist() {
        for t in TASKS {
            if let Do::Tool(name, args) = t.what {
                assert!(crate::mcp::tools::find(name).is_some(), "{name}");
                assert!(serde_json::from_str::<serde_json::Value>(args).is_ok(), "{args}");
            }
        }
    }
}
