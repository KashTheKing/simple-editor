//! ---- ws:visualizer-helper ----
//! Lyrics Karaoke timing: the user's exact lyrics onto the song's vocal onsets (a port of
//! `plans/visualizer-helper/hsv_lyrics.py` that needs no Python and no transcript).
//!
//! Each word gets an estimated start: from the song's transcript when the app has one (in-order fuzzy
//! match), else by spreading the syllables over the stretches where the voice sounds. Then a monotonic
//! dynamic programme puts every word on a vocal-stem syllable onset, weighing how far the onset is from the
//! estimate and how far its index is from the word's syllable count (one onset per syllable), and leaves a
//! word on its estimate only when no onset fits. Lines appear 0.35 s before their first word, end when the
//! voice stops (the last word is held, so "NOWWWW" fills slowly) and never overlap.

use crate::model::SongViz;

/// A lead-in before a line's first word, and the fade after the voice stops.
pub const LEAD: f64 = 0.35;
pub const TAIL: f64 = 0.3;

/// The lyrics as on-screen lines (upper-case), plus the index of the first line after a `[... DROP ...]`
/// header (the song's structure: no drop before the line ahead of it has ended).
pub fn parse(text: &str) -> (Vec<String>, Option<usize>) {
    let mut lines = Vec::new();
    let mut drop_before = None;
    for raw in text.lines() {
        let s = raw.trim().trim_matches(|c| matches!(c, '"' | '“' | '”')).trim();
        if s.starts_with('[') && s.ends_with(']') {
            if s.to_uppercase().contains("DROP") && drop_before.is_none() && !lines.is_empty() {
                drop_before = Some(lines.len());
            }
            continue;
        }
        if s.is_empty() {
            continue;
        }
        lines.push(s.split_whitespace().collect::<Vec<_>>().join(" ").to_uppercase());
    }
    (lines, drop_before)
}

/// The Greek look: A -> Λ, E -> Σ, O -> Θ, Y -> Ψ.
pub fn greek(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A' => 'Λ',
            'E' => 'Σ',
            'O' => 'Θ',
            'Y' => 'Ψ',
            c => c,
        })
        .collect()
}

/// Letters and digits only, upper-case: what words are compared on.
fn norm(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_uppercase).collect()
}

/// Syllables in a word: groups of vowels (a final silent E dropped), at least one.
pub fn syllables(w: &str) -> usize {
    let w: Vec<char> = norm(w).chars().collect();
    let vowel = |c: char| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U' | 'Y');
    let mut n = 0;
    let mut prev = false;
    for (i, &c) in w.iter().enumerate() {
        let v = vowel(c) && !(c == 'E' && i + 1 == w.len() && i >= 2 && !vowel(w[i - 1]));
        if v && !prev {
            n += 1;
        }
        prev = v;
    }
    n.max(1)
}

/// A transcript word (source seconds of the song).
#[derive(Clone, Debug)]
pub struct Said {
    pub text: String,
    pub start: f64,
}

/// One timed line (source seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub words: Vec<String>,
    pub starts: Vec<f64>,
    pub ends: Vec<f64>,
    pub clip_start: f64,
    pub clip_end: f64,
    /// Silver before the first drop, gold from it.
    pub gold: bool,
    /// The line that ends into the biggest drop: wrapped round a cylinder.
    pub giant: bool,
}

/// Similarity of two normalised words, 0..1 (2 x the longest common prefix-or-suffix run / total).
fn similar(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    // longest common subsequence: what difflib's ratio measures, close enough for single words
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in 0..a.len() {
        for j in 0..b.len() {
            dp[i + 1][j + 1] = if a[i] == b[j] { dp[i][j] + 1 } else { dp[i][j + 1].max(dp[i + 1][j]) };
        }
    }
    2.0 * dp[a.len()][b.len()] as f64 / (a.len() + b.len()) as f64
}

/// The sung stretches (gaps under 0.6 s already bridged by the analysis).
fn spans(v: &SongViz) -> Vec<(f64, f64)> {
    v.voiced.iter().map(|s| (s[0] as f64, s[1] as f64)).filter(|(a, b)| b > a).collect()
}

/// Position `u` seconds into the concatenated `spans`.
fn at(spans: &[(f64, f64)], mut u: f64) -> f64 {
    for &(a, b) in spans {
        if u <= b - a {
            return a + u;
        }
        u -= b - a;
    }
    spans.last().map_or(u, |s| s.1)
}

/// Each word's estimated start: transcript anchors where words match, the rest spread by syllables
/// (between anchors, or over the sung stretches).
fn estimates(words: &[String], v: &SongViz, said: &[Said]) -> Vec<f64> {
    let syl: Vec<f64> = words.iter().map(|w| syllables(w) as f64).collect();
    let mut est: Vec<Option<f64>> = vec![None; words.len()];
    if !said.is_empty() {
        // in-order match: longest common subsequence over similar words
        let a: Vec<String> = words.iter().map(|w| norm(w)).collect();
        let b: Vec<String> = said.iter().map(|w| norm(&w.text)).collect();
        let (n, m) = (a.len(), b.len());
        let mut dp = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i][j] = if similar(&a[i], &b[j]) >= 0.5 { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if similar(&a[i], &b[j]) >= 0.5 && dp[i][j] == dp[i + 1][j + 1] + 1 {
                est[i] = Some(said[j].start);
                i += 1;
                j += 1;
            } else if dp[i + 1][j] >= dp[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    let sp = spans(v);
    let total: f64 = sp.iter().map(|(a, b)| b - a).sum();
    if est.iter().all(Option::is_none) {
        // no anchors: the syllables spread evenly over the singing
        let all: f64 = syl.iter().sum();
        let mut acc = 0.0;
        return syl
            .iter()
            .map(|s| {
                let t = if sp.is_empty() { acc * 0.3 } else { at(&sp, total * acc / all.max(1.0)) };
                acc += s;
                t
            })
            .collect();
    }
    // between anchors: by syllables (before the first / after the last: 0.3 s a syllable)
    let mut out = vec![0.0; words.len()];
    let mut k = 0;
    while k < words.len() {
        if let Some(t) = est[k] {
            out[k] = t;
            k += 1;
            continue;
        }
        let j = (k..words.len()).find(|&q| est[q].is_some()).unwrap_or(words.len());
        let before = if k > 0 { Some(out[k - 1]) } else { None };
        let after = est.get(j).copied().flatten();
        let run: f64 = syl[k..j].iter().sum();
        match (before, after) {
            (Some(t0), Some(t1)) => {
                let lead = syl[k - 1];
                let step = (t1 - t0) / (run + lead);
                let mut acc = lead;
                for q in k..j {
                    out[q] = t0 + step * acc;
                    acc += syl[q];
                }
            }
            (Some(t0), None) => {
                let mut t = t0 + 0.3 * syl[k - 1];
                for q in k..j {
                    out[q] = t;
                    t += 0.3 * syl[q];
                }
            }
            (None, Some(t1)) => {
                let mut t = t1 - 0.3 * run;
                for q in k..j {
                    out[q] = t.max(0.0);
                    t += 0.3 * syl[q];
                }
            }
            (None, None) => {}
        }
        k = j;
    }
    out
}

/// Put `words` (in order) onto onsets: the cheapest monotonic assignment where a word on onset `j` costs
/// its distance in seconds from its estimate plus half its distance in onsets from where its syllable count
/// says it should be, and a word without an onset costs `SKIP`. Returns each word's onset index (or None).
fn assign(est: &[f64], syl_at: &[f64], onsets: &[f64], index_weight: f64) -> Vec<Option<usize>> {
    const SKIP: f64 = 1.5;
    let (k_n, j_n) = (est.len(), onsets.len());
    if k_n == 0 {
        return Vec::new();
    }
    // expected onset index of each word: its syllable position, scaled when the counts differ
    let total = syl_at.last().copied().unwrap_or(0.0) + 1.0;
    let scale = if j_n as f64 > total { 1.0 } else { (j_n as f64 / total).max(1e-9) };
    let first = onsets.iter().position(|&o| o >= est[0] - 1.0).unwrap_or(0) as f64;
    let expect: Vec<f64> = syl_at.iter().map(|&s| first + s * scale).collect();
    // state s = onsets 0..s-1 may no longer be used (the last word on an onset was on s-1; 0 = none yet).
    // cost[s] after each word; choice[k][s] = how word k reached s: -1 skipped (same state), else the state
    // before it (word k sat on onset s-1).
    let inf = f64::INFINITY;
    let mut cost = vec![inf; j_n + 1];
    cost[0] = 0.0;
    let mut choice = vec![vec![-1i64; j_n + 1]; k_n];
    for k in 0..k_n {
        let mut next: Vec<f64> = cost.iter().map(|c| c + SKIP).collect();
        let mut best = (inf, 0usize); // cheapest state <= j so far
        for j in 0..j_n {
            if cost[j] < best.0 {
                best = (cost[j], j);
            }
            if !best.0.is_finite() {
                continue;
            }
            let c = best.0 + (onsets[j] - est[k]).abs() + index_weight * (j as f64 - expect[k]).abs();
            if c < next[j + 1] {
                next[j + 1] = c;
                choice[k][j + 1] = best.1 as i64;
            }
        }
        cost = next;
    }
    let mut s = (0..=j_n).min_by(|&a, &b| cost[a].total_cmp(&cost[b]).then(a.cmp(&b))).unwrap_or(0);
    let mut out = vec![None; k_n];
    for k in (0..k_n).rev() {
        let c = choice[k][s];
        if c >= 0 {
            out[k] = Some(s - 1);
            s = c as usize;
        }
    }
    out
}

/// Time `lines` against the analysis `v` (and the transcript `said`, when the app has one).
pub fn time(lines: &[String], v: &SongViz, said: &[Said]) -> Vec<Line> {
    let words: Vec<String> = lines.iter().flat_map(|l| l.split_whitespace().map(String::from)).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let est = estimates(&words, v, said);
    let mut syl_at = Vec::with_capacity(words.len());
    let mut acc = 0.0;
    for w in &words {
        syl_at.push(acc);
        acc += syllables(w) as f64;
    }
    // only onsets where the voice sounds (bridged stretches), from the first word's neighbourhood on
    let sp = spans(v);
    let onsets: Vec<f64> = v.onsets.iter().map(|&o| o as f64).filter(|&o| sp.is_empty() || sp.iter().any(|&(a, b)| o >= a - 0.15 && o <= b)).collect();
    let pick = assign(&est, &syl_at, &onsets, if said.is_empty() { 0.5 } else { 0.1 });
    let mut starts = Vec::with_capacity(words.len());
    let mut prev = -1.0f64;
    for (k, p) in pick.iter().enumerate() {
        let t = match p {
            Some(j) => onsets[*j],
            None => est[k],
        };
        let t = t.max(prev + 0.1);
        starts.push(t);
        prev = t;
    }
    // per line
    let drops: Vec<f64> = v.drops.iter().map(|&d| d as f64).collect();
    let mut out: Vec<Line> = Vec::new();
    let mut idx = 0;
    let line_words: Vec<Vec<String>> = lines.iter().map(|l| l.split_whitespace().map(String::from).collect()).collect();
    for (li, ws) in line_words.iter().enumerate() {
        let m = ws.len();
        if m == 0 {
            continue;
        }
        let s = starts[idx..idx + m].to_vec();
        idx += m;
        let next_first = (li + 1..line_words.len()).find(|&q| !line_words[q].is_empty()).map(|_| starts[idx]);
        let nxt = next_first.map_or(f64::INFINITY, |t| t - LEAD);
        let last = *s.last().unwrap_or(&0.0);
        // held until the voice stops (its stretch ends), within the next line's lead-in and 12 s
        let held = sp.iter().find(|&&(a, b)| last + 0.08 >= a && last + 0.08 <= b).map_or(last + 0.6, |&(_, b)| b);
        let end_last = held.min(nxt).min(last + 12.0).max(last + 0.25);
        let mut ends: Vec<f64> = s.iter().skip(1).copied().collect();
        ends.push(end_last);
        let clip_start = (s[0] - LEAD).max(out.last().map_or(0.0, |l| l.clip_end)).max(0.0);
        let clip_end = (end_last + TAIL).min(nxt).max(clip_start + 0.3);
        out.push(Line {
            text: lines[li].clone(),
            words: ws.clone(),
            starts: s.iter().map(|&t| r3(t)).collect(),
            ends: ends.iter().map(|&t| r3(t)).collect(),
            clip_start: r3(clip_start),
            clip_end: r3(clip_end),
            gold: drops.first().is_some_and(|&d| clip_start >= d - 0.5),
            giant: false,
        });
    }
    // the line that ends right into the biggest drop wraps round a cylinder
    if drops.len() >= 2 {
        let big = drops[drops.len() - 1];
        if let Some(k) = out.iter().rposition(|l| l.clip_end >= big - 3.5 && l.clip_end <= big + 0.6) {
            out[k].giant = true;
        }
    }
    out
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The `fill` keys of the Lyric Chrome shader for `line` (clip-local seconds, value = how far across the
/// text box the fill has reached): word edges from `edges`, the x of each character boundary of the shown
/// text in the text bitmap (`chars + 1` values) over its width `w`.
pub fn fill_keys(line: &Line, shown: &str, edges: &[f32], w: f32) -> Vec<(f64, f64)> {
    let w = w.max(1.0) as f64;
    let chars: Vec<char> = shown.chars().collect();
    let x = |i: usize| edges.get(i.min(edges.len().saturating_sub(1))).copied().unwrap_or(0.0) as f64;
    // the char span of each word of the shown text
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let a = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        spans.push((a, i));
    }
    let cs = line.clip_start;
    let mut keys = vec![(0.0, r4(x(spans.first().map_or(0, |s| s.0)) / w))];
    for (k, &(a, b)) in spans.iter().enumerate() {
        let (Some(&s), Some(&e)) = (line.starts.get(k), line.ends.get(k)) else { break };
        let e2 = line.starts.get(k + 1).map_or(e, |&n| e.min(n));
        keys.push((r3(s - cs), r4(x(a) / w)));
        keys.push((r3((e2 - cs - 0.01).max(s - cs + 0.005)), r4(x(b) / w)));
    }
    keys
}

fn r4(v: f64) -> f64 {
    (v * 10000.0).round() / 10000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One sung stretch 6.0-10.0 s with a syllable onset on each sung syllable.
    fn song(onsets: &[f64]) -> SongViz {
        SongViz { voiced: vec![[6.0, 10.0]], onsets: onsets.iter().map(|&o| o as f32).collect(), drops: vec![20.0], ..Default::default() }
    }

    #[test]
    fn syllables_are_counted() {
        assert_eq!(syllables("RISE"), 1);
        assert_eq!(syllables("THUNDER"), 2);
        assert_eq!(syllables("NOWWWW"), 1);
        assert_eq!(syllables("OLYMPUS"), 3);
    }

    #[test]
    fn lyrics_words_land_on_the_vocal_onsets() {
        // RISE UP NOW / FEEL THE THUN-DER ROLL ON: 9 syllables, 9 onsets that are not evenly spaced
        let onsets = [6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 8.75, 9.0, 9.5];
        let (lines, _) = parse("[Verse]\nRise up now\n\n\"Feel the thunder roll on\"\n");
        assert_eq!(lines, vec!["RISE UP NOW", "FEEL THE THUNDER ROLL ON"]);
        let t = time(&lines, &song(&onsets), &[]);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].starts, vec![6.0, 6.5, 7.0]);
        assert_eq!(t[1].starts, vec![7.5, 8.0, 8.5, 9.0, 9.5]);
        // a line shows 0.35 s before its first word and never overlaps the next
        assert_eq!(t[0].clip_start, 5.65);
        assert!(t[0].clip_end <= t[1].clip_start + 1e-9);
        assert!(t[0].clip_end <= 7.5 - LEAD + 1e-9);
        // the last word is held to the end of the singing
        assert!(t[1].ends[4] >= 9.9, "{:?}", t[1].ends);
        assert!(!t[0].gold && !t[0].giant);
    }

    #[test]
    fn a_stray_onset_is_skipped_and_a_transcript_anchors_words() {
        let onsets = [6.0, 6.5, 6.62, 7.0, 7.5, 8.0, 8.5, 8.75, 9.0, 9.5];
        let lines = vec!["RISE UP NOW".to_string(), "FEEL THE THUNDER ROLL ON".to_string()];
        let said: Vec<Said> = [("rise", 6.02), ("up", 6.48), ("now", 7.03), ("feel", 7.46), ("the", 8.02), ("thunder", 8.49), ("roll", 9.01), ("on", 9.52)]
            .iter()
            .map(|&(w, t)| Said { text: w.into(), start: t })
            .collect();
        let t = time(&lines, &song(&onsets), &said);
        let all: Vec<f64> = t.iter().flat_map(|l| l.starts.clone()).collect();
        assert_eq!(all, vec![6.0, 6.5, 7.0, 7.5, 8.0, 8.5, 9.0, 9.5]);
    }

    #[test]
    fn fill_keys_follow_the_word_edges() {
        let line = Line {
            text: "AB CD".into(),
            words: vec!["AB".into(), "CD".into()],
            starts: vec![1.0, 2.0],
            ends: vec![2.0, 3.0],
            clip_start: 0.65,
            clip_end: 3.3,
            gold: false,
            giant: false,
        };
        // 5 chars, 10 px each, a 6 px margin: boundaries 6, 16, 26, 36, 46, 56 of a 62 px bitmap
        let edges: Vec<f32> = (0..6).map(|i| 6.0 + 10.0 * i as f32).collect();
        let k = fill_keys(&line, "AB CD", &edges, 62.0);
        assert_eq!(k[0], (0.0, 0.0968));
        assert_eq!(k[1], (0.35, 0.0968));
        assert_eq!(k[2], (1.34, 0.4194));
        assert_eq!(k[3], (1.35, 0.5806));
        assert_eq!(k[4], (2.34, 0.9032));
    }

    #[test]
    fn greek_look_maps_the_vowels() {
        assert_eq!(greek("OLYMPUS RISE"), "ΘLΨMPUS RISΣ");
    }
}
