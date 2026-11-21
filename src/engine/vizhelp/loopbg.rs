//! ---- ws:visualizer-helper ----
//! Background Loop: a low-frame-rate GIF, clip or still into a seamless 60 fps loop (a port of
//! `hsv_background.py`). Motion-interpolated to 60 fps (minterpolate, mci), cover-cropped to the project
//! size, slowed to `seconds`, then played forward and back (ping-pong) so the loop point never jumps (plain
//! loops jumped visibly and crossfades ghosted). A still gets a slow 6% push-in instead, ping-ponged the same
//! way. Two ffmpeg runs; the argument lists are built here so they can be tested.

/// The first pass: `input` -> `one` (the one-way pass, crf 18).
pub fn pass1(input: &str, one: &str, still: bool, src_secs: f64, seconds: f64, w: u32, h: u32) -> Vec<String> {
    let (w, h) = (w.max(16) & !1, h.max(16) & !1);
    let seconds = seconds.clamp(1.0, 60.0);
    let mut a: Vec<String> = ["-v", "error", "-y"].iter().map(|s| s.to_string()).collect();
    if still {
        let n = (seconds * 60.0).round() as u32;
        a.extend(["-loop".into(), "1".into(), "-i".into(), input.into()]);
        a.extend([
            "-vf".into(),
            format!("scale={}:-2,zoompan=z='1+0.06*on/{n}':x='iw/2-iw/zoom/2':y='ih/2-ih/zoom/2':d={n}:s={w}x{h}:fps=60", w * 2),
            "-frames:v".into(),
            n.to_string(),
        ]);
    } else {
        // stretch the source to `seconds`, then invent the in-between frames
        let stretch = seconds / src_secs.max(0.05);
        a.extend(["-i".into(), input.into()]);
        a.extend([
            "-vf".into(),
            format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setpts=PTS*{stretch:.5},minterpolate=fps=60:mi_mode=mci:mc_mode=aobmc:vsbmc=1,format=yuv420p"),
            "-t".into(),
            format!("{seconds:.3}"),
            "-an".into(),
        ]);
    }
    a.extend(["-pix_fmt", "yuv420p", "-c:v", "libx264", "-crf", "18", one].iter().map(|s| s.to_string()));
    a
}

/// The second pass: `one` forward then reversed -> `out` (crf 21, about 12 Mbps at 1080p).
pub fn pass2(one: &str, out: &str) -> Vec<String> {
    ["-v", "error", "-y", "-i", one, "-filter_complex", "[0:v]split[a][b];[b]reverse[r];[a][r]concat=n=2:v=1[v]", "-map", "[v]", "-c:v", "libx264", "-crf", "21", "-pix_fmt", "yuv420p", "-movflags", "+faststart", out]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gif_is_interpolated_and_ping_ponged() {
        let a = pass1("in.gif", "one.mp4", false, 2.0, 8.0, 1920, 1080);
        let vf = &a[a.iter().position(|s| s == "-vf").unwrap() + 1];
        assert!(vf.contains("crop=1920:1080"));
        assert!(vf.contains("setpts=PTS*4.00000"));
        assert!(vf.contains("minterpolate=fps=60:mi_mode=mci:mc_mode=aobmc:vsbmc=1"));
        assert_eq!(a.last().unwrap(), "one.mp4");
        let b = pass2("one.mp4", "out.mp4");
        assert!(b.iter().any(|s| s.contains("reverse") && s.contains("concat=n=2")));
        let s = pass1("in.png", "one.mp4", true, 0.0, 8.0, 1280, 720);
        assert!(s.iter().any(|x| x.contains("zoompan") && x.contains("s=1280x720")));
        assert!(s.contains(&"480".to_string()));
    }
}
