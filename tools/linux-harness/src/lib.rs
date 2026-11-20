#![allow(dead_code, unused)]
pub mod media {
    #[derive(Clone, Default)] pub struct Frame { pub width: u32, pub height: u32, pub rgba: Vec<u8> }
    impl Frame { pub fn new(w: u32, h: u32) -> Self { Frame { width: w, height: h, rgba: vec![0; (w*h*4) as usize] } } }
    pub mod bg { pub fn lower_thread() {} }
    pub enum Backend { Auto }
    pub trait VideoSource { fn size(&self) -> (u32, u32); fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool; }
    pub fn open_video(_: &str, _: Backend) -> Result<Box<dyn VideoSource>, String> { Err("harness".into()) }
    pub mod ffpipe {
        pub fn ffprobe_exe() -> Option<std::path::PathBuf> { Some("ffprobe".into()) }
        pub fn ffmpeg_exe() -> Option<std::path::PathBuf> { Some("ffmpeg".into()) }
        pub fn command(p: &std::path::Path) -> std::process::Command { std::process::Command::new(p) }
    }
    pub use crate::io_override_src as io_override;
}
pub mod engine {
    pub mod html { pub fn tie_to_us(_: &std::process::Child) {} }
    pub use crate::{media_info_src as media_info, netstream_src as netstream, shuttle_src as shuttle};
}
// the app's own files (`#[path]` at the crate root: a path inside an inline module would go through a
// `src/media/` directory that doesn't exist here)
#[path = "../../../src/media/io_override.rs"] pub mod io_override_src;
#[path = "../../../src/engine/media_info.rs"] pub mod media_info_src;
#[path = "../../../src/engine/netstream.rs"] pub mod netstream_src;
#[path = "../../../src/engine/shuttle.rs"] pub mod shuttle_src;
pub mod hotkeys { pub struct Action; impl Action { pub fn from_id(_: &str) -> Option<()> { Some(()) } } }
