//! ---- ws:music-burn ----
//! Windows disc burning (IMAPI2) over the `windows` crate's COM bindings: drives, the disc in them,
//! erasing, a file-system image (ISO 9660 + Joliet + UDF) written to a `.iso` or a drive, and audio
//! tracks track-at-once. Every call runs on the caller's thread in a multithreaded apartment (the burn
//! job's own thread); progress comes from IMAPI's `Update` events, and Cancel is answered from inside
//! them (`CancelWrite` / `CancelAddTrack`, the documented way).
//!
//! IMAPI2 has no "simulate" switch (IMAPI v1's `RecordDisc(bSimulate)` is gone): `test_only` does every
//! check a real burn does - the drive opens, the disc is blank / writable and big enough, the image is
//! built - and stops before writing.

use super::{BurnOptions, DataItem, DiscSize, TrackPlace, AUDIO_SECTOR, DATA_BLOCK};
use crate::engine::export::Progress;
use std::path::Path;
use windows::core::{IUnknown, Interface, BSTR, GUID, HRESULT};
use windows::Win32::Foundation::{VARIANT_BOOL, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::Storage::Imapi::*;
use windows::Win32::System::Com::*;

const CLIENT: &str = "Simple Editor";

fn vb(b: bool) -> VARIANT_BOOL {
    if b {
        VARIANT_TRUE
    } else {
        VARIANT_FALSE
    }
}

/// A plain-words error for an IMAPI failure.
fn msg(e: windows::core::Error) -> String {
    let h = e.code();
    let text = match h {
        IMAPI_E_MEDIUM_NOTPRESENT => "There is no disc in the drive.",
        IMAPI_E_MEDIUM_INVALIDTYPE | IMAPI_E_WRONGDISC => "The disc in the drive can't be written (wrong type, or not blank).",
        IMAPI_E_DISCFULL | IMAPI_E_TRACK_NOT_BIG_ENOUGH => "The disc is full.",
        IMAPI_E_DEVICE_NOTPRESENT | IMAPI_E_DEVICE_NOTACCESSIBLE => "The drive is not available.",
        IMAPI_E_DEVICE_STILL_IN_USE | IMAPI_E_ALREADYOPEN | IMAPI_E_STASHINUSE => "Another program is using the drive.",
        IMAPI_E_USERABORT => crate::engine::export::CANCELLED,
        IMAPI_E_LOSS_OF_STREAMING => "The drive ran out of data while writing (buffer underrun).",
        IMAPI_E_FILEACCESS => "A file could not be read.",
        _ => "",
    };
    // IMAPI2FS's own codes (0xC0AAB1xx) - the one users meet is "image too big"
    let code = h.0 as u32;
    if code == 0xC0AA_B120 || code == 0xC0AA_B15C {
        return "Too big for this disc.".into();
    }
    if code == 0x8000_4005 || code == 0xC0AA_0002 {
        return "Burn cancelled.".into();
    }
    if text.is_empty() {
        format!("Disc burning failed: {} (0x{code:08X})", e.message().trim())
    } else {
        text.to_string()
    }
}

/// COM for this thread (multithreaded apartment), released on drop when we started it.
struct Com(bool);
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}
fn com() -> Com {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    Com(hr.is_ok())
}

fn create<T: Interface>(clsid: &GUID) -> Result<T, String> {
    unsafe { CoCreateInstance(clsid, None, CLSCTX_ALL) }.map_err(|e| {
        format!("Windows disc burning (IMAPI2) is not available: {}", e.message().trim())
    })
}

/// A burner.
#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    /// IMAPI's unique id (pass back to the other calls).
    pub id: String,
    /// "HL-DT-ST DVDRAM GP65NB60".
    pub name: String,
}

/// Every drive Windows can burn with. Empty when there is none (not an error).
pub fn drives() -> Result<Vec<Drive>, String> {
    let _c = com();
    let m: IDiscMaster2 = create(&MsftDiscMaster2)?;
    unsafe {
        if !m.IsSupportedEnvironment().map_err(msg)?.as_bool() {
            return Ok(Vec::new());
        }
        let n = m.Count().map_err(msg)?;
        let mut out = Vec::new();
        for i in 0..n {
            let id = m.get_Item(i).map_err(msg)?;
            let r: IDiscRecorder2 = create(&MsftDiscRecorder2)?;
            if r.InitializeDiscRecorder(&id).is_err() {
                continue;
            }
            let v = r.VendorId().map(|s| s.to_string()).unwrap_or_default();
            let p = r.ProductId().map(|s| s.to_string()).unwrap_or_default();
            out.push(Drive { id: id.to_string(), name: format!("{} {}", v.trim(), p.trim()).trim().to_string() });
        }
        Ok(out)
    }
}

fn recorder(id: &str) -> Result<IDiscRecorder2, String> {
    let r: IDiscRecorder2 = create(&MsftDiscRecorder2)?;
    unsafe { r.InitializeDiscRecorder(&BSTR::from(id)) }.map_err(|_| "That drive is gone; pick it again.".to_string())?;
    Ok(r)
}

/// The disc in a drive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub present: bool,
    pub kind: String,
    pub blank: bool,
    pub rewritable: bool,
    pub cd: bool,
    pub appendable: bool,
    pub write_protected: bool,
    pub total_sectors: u64,
    pub free_sectors: u64,
    /// Write speeds the drive offers for this disc, in sectors per second.
    pub speeds: Vec<i32>,
}

impl MediaInfo {
    /// One line for the dialog.
    pub fn describe(&self) -> String {
        if !self.present {
            return "No disc".into();
        }
        let state = if self.write_protected {
            "write-protected"
        } else if self.blank {
            "blank"
        } else if self.appendable {
            "has data, can add more"
        } else {
            "full or closed"
        };
        let free = if self.cd {
            super::clock(self.free_sectors as f64 / 75.0) + " free"
        } else {
            format!("{} MB free", self.free_sectors * DATA_BLOCK / 1_000_000)
        };
        format!("{} · {state} · {free}", self.kind)
    }
    pub fn check_audio(&self) -> Result<(), String> {
        if !self.present {
            return Err("There is no disc in the drive. Insert a blank CD-R.".into());
        }
        if !self.cd {
            return Err(format!("An audio CD needs a CD-R or CD-RW; the drive has a {}.", self.kind));
        }
        if !self.blank {
            return Err(if self.rewritable {
                "The CD-RW is not blank: erase it first.".into()
            } else {
                "The CD-R is not blank: insert a blank one.".into()
            });
        }
        Ok(())
    }
    pub fn check_data(&self) -> Result<(), String> {
        if !self.present {
            return Err("There is no disc in the drive.".into());
        }
        if self.write_protected {
            return Err("The disc is write-protected.".into());
        }
        if !self.blank {
            return Err(if self.rewritable {
                "The disc is not blank: erase it first.".into()
            } else {
                "The disc is not blank: insert a blank one.".into()
            });
        }
        Ok(())
    }
}

fn kind_name(t: IMAPI_MEDIA_PHYSICAL_TYPE) -> (&'static str, bool, bool) {
    // (name, rewritable, cd)
    match t {
        IMAPI_MEDIA_TYPE_CDR => ("CD-R", false, true),
        IMAPI_MEDIA_TYPE_CDRW => ("CD-RW", true, true),
        IMAPI_MEDIA_TYPE_CDROM => ("CD-ROM", false, true),
        IMAPI_MEDIA_TYPE_DVDPLUSR => ("DVD+R", false, false),
        IMAPI_MEDIA_TYPE_DVDPLUSRW => ("DVD+RW", true, false),
        IMAPI_MEDIA_TYPE_DVDPLUSR_DUALLAYER => ("DVD+R DL", false, false),
        IMAPI_MEDIA_TYPE_DVDPLUSRW_DUALLAYER => ("DVD+RW DL", true, false),
        IMAPI_MEDIA_TYPE_DVDDASHR => ("DVD-R", false, false),
        IMAPI_MEDIA_TYPE_DVDDASHRW => ("DVD-RW", true, false),
        IMAPI_MEDIA_TYPE_DVDDASHR_DUALLAYER => ("DVD-R DL", false, false),
        IMAPI_MEDIA_TYPE_DVDRAM => ("DVD-RAM", true, false),
        IMAPI_MEDIA_TYPE_DVDROM => ("DVD-ROM", false, false),
        IMAPI_MEDIA_TYPE_BDR => ("BD-R", false, false),
        IMAPI_MEDIA_TYPE_BDRE => ("BD-RE", true, false),
        IMAPI_MEDIA_TYPE_BDROM => ("BD-ROM", false, false),
        _ => ("disc", false, false),
    }
}

/// What is in drive `id` right now.
pub fn media_info(id: &str) -> Result<MediaInfo, String> {
    let _c = com();
    let r = recorder(id)?;
    let f: IDiscFormat2Data = create(&MsftDiscFormat2Data)?;
    unsafe {
        if !f.IsCurrentMediaSupported(&r).map(|b| b.as_bool()).unwrap_or(false) {
            // a blank CD-R also takes track-at-once; anything else: no disc / not writable
            let tao: IDiscFormat2TrackAtOnce = create(&MsftDiscFormat2TrackAtOnce)?;
            if tao.IsCurrentMediaSupported(&r).map(|b| b.as_bool()).unwrap_or(false) {
                tao.SetRecorder(&r).map_err(msg)?;
                let t = tao.CurrentPhysicalMediaType().unwrap_or(IMAPI_MEDIA_TYPE_UNKNOWN);
                let (k, rw, cd) = kind_name(t);
                return Ok(MediaInfo {
                    present: true,
                    kind: k.into(),
                    blank: tao.NumberOfExistingTracks().unwrap_or(1) == 0,
                    rewritable: rw,
                    cd,
                    total_sectors: tao.TotalSectorsOnMedia().unwrap_or(0).max(0) as u64,
                    free_sectors: tao.FreeSectorsOnMedia().unwrap_or(0).max(0) as u64,
                    ..Default::default()
                });
            }
            return Ok(MediaInfo::default());
        }
        f.SetRecorder(&r).map_err(msg)?;
        let t = f.CurrentPhysicalMediaType().unwrap_or(IMAPI_MEDIA_TYPE_UNKNOWN);
        let (k, rw, cd) = kind_name(t);
        let st = f.CurrentMediaStatus().map(|s| s.0).unwrap_or(0);
        let speeds = read_speeds(f.SupportedWriteSpeeds().ok());
        Ok(MediaInfo {
            present: true,
            kind: k.into(),
            blank: f.MediaHeuristicallyBlank().map(|b| b.as_bool()).unwrap_or(false),
            rewritable: rw,
            cd,
            appendable: st & IMAPI_FORMAT2_DATA_MEDIA_STATE_APPENDABLE.0 != 0,
            write_protected: st & IMAPI_FORMAT2_DATA_MEDIA_STATE_WRITE_PROTECTED.0 != 0,
            total_sectors: f.TotalSectorsOnMedia().unwrap_or(0).max(0) as u64,
            free_sectors: f.FreeSectorsOnMedia().unwrap_or(0).max(0) as u64,
            speeds,
        })
    }
}

/// A SAFEARRAY of VARIANT ints (write speeds).
fn read_speeds(sa: Option<*mut SAFEARRAY>) -> Vec<i32> {
    use windows::Win32::System::Ole::{SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound};
    use windows::Win32::System::Variant::{VariantClear, VARIANT};
    let Some(sa) = sa.filter(|p| !p.is_null()) else { return Vec::new() };
    let mut out = Vec::new();
    unsafe {
        let (lo, hi) = (SafeArrayGetLBound(sa, 1).unwrap_or(0), SafeArrayGetUBound(sa, 1).unwrap_or(-1));
        for i in lo..=hi {
            let mut v = VARIANT::default();
            if SafeArrayGetElement(sa, &i, &mut v as *mut _ as *mut _).is_ok() {
                out.push(v.Anonymous.Anonymous.Anonymous.lVal);
                let _ = VariantClear(&mut v);
            }
        }
        let _ = SafeArrayDestroy(sa);
    }
    out.retain(|&n| n > 0);
    out.sort_unstable();
    out.dedup();
    out
}

pub fn eject(id: &str) -> Result<(), String> {
    let _c = com();
    unsafe { recorder(id)?.EjectMedia() }.map_err(msg)
}

/// Quick-erase a rewritable disc (about a minute).
pub fn erase(id: &str, prog: &Progress) -> Result<(), String> {
    let _c = com();
    let r = recorder(id)?;
    let e: IDiscFormat2Erase = create(&MsftDiscFormat2Erase)?;
    unsafe {
        if !e.IsCurrentMediaSupported(&r).map(|b| b.as_bool()).unwrap_or(false) {
            return Err("The disc in the drive is not rewritable (or there is none).".into());
        }
        e.SetRecorder(&r).map_err(msg)?;
        e.SetClientName(&BSTR::from(CLIENT)).map_err(msg)?;
        e.SetFullErase(VARIANT_FALSE).map_err(msg)?;
        prog.set(0.0, "Erasing");
        let sink = sink(SinkKind::Erase, DDiscFormat2EraseEvents::IID, prog);
        let cookie = advise(&e, &DDiscFormat2EraseEvents::IID, &sink);
        let r = e.EraseMedia().map_err(msg);
        unadvise(cookie);
        r
    }
}

// ------------------------------------------------------------------------------------------------ events

/// The job's progress, handed to an event sink that lives only while a blocking IMAPI call runs (the
/// caller unadvises before returning), so the pointer never outlives the `&Progress` it came from.
#[derive(Clone, Copy)]
struct ProgPtr(*const Progress);
impl ProgPtr {
    fn get(&self) -> &Progress {
        unsafe { &*self.0 }
    }
}

/// What an `Update` event is about.
#[derive(Clone, Copy, PartialEq)]
enum SinkKind {
    Data,
    Tao,
    Erase,
}

/// A hand-made COM event sink (IUnknown + IDispatch + the event's `Update`). Written out instead of
/// `#[implement]`, whose expansion needs the `windows-core` crate as a direct dependency.
#[repr(C)]
struct Sink {
    vtbl: *const SinkVtbl,
    refs: std::sync::atomic::AtomicU32,
    kind: SinkKind,
    iid: GUID,
    prog: ProgPtr,
}

type Raw = *mut core::ffi::c_void;

#[repr(C)]
struct SinkVtbl {
    query: unsafe extern "system" fn(Raw, *const GUID, *mut Raw) -> HRESULT,
    add_ref: unsafe extern "system" fn(Raw) -> u32,
    release: unsafe extern "system" fn(Raw) -> u32,
    type_info_count: unsafe extern "system" fn(Raw, *mut u32) -> HRESULT,
    type_info: unsafe extern "system" fn(Raw, u32, u32, *mut Raw) -> HRESULT,
    ids_of_names: unsafe extern "system" fn(Raw, *const GUID, *const Raw, u32, u32, *mut i32) -> HRESULT,
    invoke: unsafe extern "system" fn(Raw, i32, *const GUID, u32, u16, *const DISPPARAMS, Raw, Raw, *mut u32) -> HRESULT,
    /// `Update(object, progress)` (data / track-at-once) or `Update(object, elapsed, total)` (erase): on
    /// x64 every argument after `this` is passed in its own pointer-sized slot.
    update: unsafe extern "system" fn(Raw, Raw, isize, isize) -> HRESULT,
}

const E_NOINTERFACE: HRESULT = HRESULT(0x8000_4002_u32 as i32);
const E_NOTIMPL: HRESULT = HRESULT(0x8000_4001_u32 as i32);
const S_OK: HRESULT = HRESULT(0);

unsafe extern "system" fn s_query(this: Raw, iid: *const GUID, out: *mut Raw) -> HRESULT {
    let s = &*(this as *const Sink);
    let iid = *iid;
    if iid == IUnknown::IID || iid == IDispatch::IID || iid == s.iid {
        s_add_ref(this);
        *out = this;
        S_OK
    } else {
        *out = std::ptr::null_mut();
        E_NOINTERFACE
    }
}
unsafe extern "system" fn s_add_ref(this: Raw) -> u32 {
    (*(this as *const Sink)).refs.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}
unsafe extern "system" fn s_release(this: Raw) -> u32 {
    let n = (*(this as *const Sink)).refs.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) - 1;
    if n == 0 {
        drop(Box::from_raw(this as *mut Sink));
    }
    n
}
unsafe extern "system" fn s_count(_: Raw, n: *mut u32) -> HRESULT {
    *n = 0;
    S_OK
}
unsafe extern "system" fn s_type_info(_: Raw, _: u32, _: u32, out: *mut Raw) -> HRESULT {
    *out = std::ptr::null_mut();
    E_NOTIMPL
}
unsafe extern "system" fn s_names(_: Raw, _: *const GUID, _: *const Raw, _: u32, _: u32, _: *mut i32) -> HRESULT {
    E_NOTIMPL
}
/// Scripting-style delivery: the same `Update`, its arguments in a DISPPARAMS (in reverse order).
unsafe extern "system" fn s_invoke(this: Raw, _: i32, _: *const GUID, _: u32, _: u16, p: *const DISPPARAMS, _: Raw, _: Raw, _: *mut u32) -> HRESULT {
    let Some(p) = p.as_ref() else { return S_OK };
    if p.cArgs < 2 || p.rgvarg.is_null() {
        return S_OK;
    }
    let args = std::slice::from_raw_parts(p.rgvarg, p.cArgs as usize);
    let n = args.len();
    let word = |v: &windows::Win32::System::Variant::VARIANT| -> isize {
        let v0 = &v.Anonymous.Anonymous;
        if v0.vt == windows::Win32::System::Variant::VT_DISPATCH {
            v0.Anonymous.pdispVal.as_ref().map_or(0, |d| d.as_raw() as isize)
        } else {
            v0.Anonymous.lVal as isize
        }
    };
    let object = word(&args[n - 1]) as Raw;
    let a = word(&args[n - 2]);
    let b = if n >= 3 { word(&args[n - 3]) } else { 0 };
    s_update(this, object, a, b)
}
unsafe extern "system" fn s_update(this: Raw, object: Raw, a: isize, b: isize) -> HRESULT {
    let s = &*(this as *const Sink);
    let prog = s.prog.get();
    match s.kind {
        SinkKind::Erase => {
            let (elapsed, total) = (a as i32, b as i32);
            let f = if total > 0 { elapsed as f32 / total as f32 } else { 0.0 };
            prog.set(f.clamp(0.0, 0.99), format!("Erasing · {} left", super::clock((total - elapsed).max(0) as f64)));
        }
        SinkKind::Data | SinkKind::Tao => {
            let raw = a as Raw;
            let progress = IDispatch::from_raw_borrowed(&raw);
            if let Some(o) = IDispatch::from_raw_borrowed(&object) {
                if s.kind == SinkKind::Data {
                    data_update(prog, o, progress);
                } else {
                    tao_update(prog, o, progress);
                }
            }
        }
    }
    S_OK
}

static SINK_VTBL: SinkVtbl = SinkVtbl {
    query: s_query,
    add_ref: s_add_ref,
    release: s_release,
    type_info_count: s_count,
    type_info: s_type_info,
    ids_of_names: s_names,
    invoke: s_invoke,
    update: s_update,
};

/// A new sink for event interface `iid`, as an owned IUnknown.
fn sink(kind: SinkKind, iid: GUID, prog: &Progress) -> IUnknown {
    let b = Box::new(Sink { vtbl: &SINK_VTBL, refs: std::sync::atomic::AtomicU32::new(1), kind, iid, prog: ProgPtr(prog) });
    unsafe { IUnknown::from_raw(Box::into_raw(b) as Raw) }
}

fn advise(src: &impl Interface, iid: &GUID, sink: &IUnknown) -> Option<(IConnectionPoint, u32)> {
    unsafe {
        let cpc: IConnectionPointContainer = src.cast().ok()?;
        let cp = cpc.FindConnectionPoint(iid).ok()?;
        let cookie = cp.Advise(sink).ok()?;
        Some((cp, cookie))
    }
}

fn unadvise(c: Option<(IConnectionPoint, u32)>) {
    if let Some((cp, cookie)) = c {
        let _ = unsafe { cp.Unadvise(cookie) };
    }
}

fn data_update(prog: &Progress, object: &IDispatch, progress: Option<&IDispatch>) {
    if prog.is_cancelled() {
        if let Ok(f) = object.cast::<IDiscFormat2Data>() {
            let _ = unsafe { f.CancelWrite() };
        }
        return;
    }
    let Some(a) = progress.and_then(|p| p.cast::<IDiscFormat2DataEventArgs>().ok()) else { return };
    unsafe {
        let action = a.CurrentAction().map(|x| x.0).unwrap_or(0);
        let (start, count, last) = (a.StartLba().unwrap_or(0), a.SectorCount().unwrap_or(1).max(1), a.LastWrittenLba().unwrap_or(0));
        let left = a.RemainingTime().unwrap_or(0);
        // actions: 0 validating, 1 formatting, 2 initializing hw, 3 calibrating, 4 writing, 5 finalizing,
        // 6 completed, 7 verifying
        let (f, what) = match action {
            4 => (0.05 + 0.85 * ((last - start).max(0) as f32 / count as f32), "Writing"),
            5 => (0.92, "Closing the disc"),
            7 => (0.95, "Verifying"),
            6 => (1.0, "Done"),
            _ => (0.02, "Preparing the drive"),
        };
        prog.set(f.clamp(0.0, 1.0), format!("{what}{}", if left > 0 { format!(" · {} left", super::clock(left as f64)) } else { String::new() }));
    }
}

/// Track-at-once progress: the status line carries the track; the fraction is set by `burn_audio`
/// between tracks.
fn tao_update(prog: &Progress, object: &IDispatch, progress: Option<&IDispatch>) {
    if prog.is_cancelled() {
        if let Ok(f) = object.cast::<IDiscFormat2TrackAtOnce>() {
            let _ = unsafe { f.CancelAddTrack() };
        }
        return;
    }
    let Some(a) = progress.and_then(|p| p.cast::<IDiscFormat2TrackAtOnceEventArgs>().ok()) else { return };
    unsafe {
        let track = a.CurrentTrackNumber().unwrap_or(0);
        let (start, count, last) = (a.StartLba().unwrap_or(0), a.SectorCount().unwrap_or(1).max(1), a.LastWrittenLba().unwrap_or(0));
        let within = ((last - start).max(0) as f32 / count as f32).clamp(0.0, 1.0);
        prog.set(prog.fraction(), format!("Writing track {track} · {:.0} %", within * 100.0));
    }
}

// ------------------------------------------------------------------------------------------------ images

fn file_stream(p: &Path) -> Result<IStream, String> {
    use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;
    let w: Vec<u16> = p.as_os_str().encode_wide_z();
    unsafe {
        SHCreateStreamOnFileEx(windows::core::PCWSTR(w.as_ptr()), (STGM_READ | STGM_SHARE_DENY_WRITE).0, 0, false, None)
    }
    .map_err(|e| format!("can't open {}: {}", p.display(), e.message().trim()))
}

trait WideZ {
    fn encode_wide_z(&self) -> Vec<u16>;
}
impl WideZ for std::ffi::OsStr {
    fn encode_wide_z(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

/// ISO / Joliet volume labels are short and plain.
pub fn volume_label(s: &str) -> String {
    let v: String = s.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '_' || *c == '-').take(15).collect();
    let v = v.trim().to_string();
    if v.is_empty() {
        "DISC".into()
    } else {
        v
    }
}

/// The file-system image for `items`. `limit`: blocks the disc holds (from the drive or the picked size).
fn build_image(volume: &str, items: &[DataItem], limit: Limit) -> Result<IFileSystemImage, String> {
    let fs: IFileSystemImage = create(&MsftFileSystemImage)?;
    unsafe {
        fs.SetFileSystemsToCreate(FsiFileSystems(FsiFileSystemISO9660.0 | FsiFileSystemJoliet.0 | FsiFileSystemUDF.0)).map_err(msg)?;
        match limit {
            Limit::Blocks(b) => fs.SetFreeMediaBlocks(b.min(i32::MAX as u64) as i32).map_err(msg)?,
            Limit::Drive(r) => fs.SetMaxMediaBlocksFromDevice(r).map_err(msg)?,
        }
        fs.SetVolumeName(&BSTR::from(volume_label(volume))).map_err(msg)?;
        let root = fs.Root().map_err(msg)?;
        for it in items {
            let dir = ensure_dir(&fs, &root, &it.dest)?;
            let r = if it.src.is_dir() {
                dir.AddTree(&BSTR::from(it.src.to_string_lossy().as_ref()), VARIANT_TRUE)
            } else {
                let name = it.src.file_name().and_then(|n| n.to_str()).ok_or("bad file name")?;
                let st = file_stream(&it.src)?;
                dir.AddFile(&BSTR::from(name), &st)
            };
            r.map_err(|e| {
                let m = msg(e);
                if m.starts_with("Too big") {
                    m
                } else {
                    format!("{}: {m}", it.src.display())
                }
            })?;
        }
    }
    Ok(fs)
}

enum Limit<'a> {
    Blocks(u64),
    Drive(&'a IDiscRecorder2),
}

/// The folder `dest` ("a/b") under the image root, made when missing.
unsafe fn ensure_dir(fs: &IFileSystemImage, root: &IFsiDirectoryItem, dest: &str) -> Result<IFsiDirectoryItem, String> {
    let mut path = String::new();
    let mut cur = root.clone();
    for part in dest.split(['/', '\\']).filter(|s| !s.is_empty()) {
        path = format!("{path}\\{part}");
        let exists = fs.Exists(&BSTR::from(path.as_str())).map(|t| t == FsiItemDirectory).unwrap_or(false);
        if !exists {
            cur.AddDirectory(&BSTR::from(part)).map_err(msg)?;
        }
        let item = cur.get_Item(&BSTR::from(part)).map_err(msg)?;
        cur = item.cast::<IFsiDirectoryItem>().map_err(msg)?;
    }
    Ok(cur)
}

/// Copy an IStream to a file, `total` bytes expected (progress), cancellable.
fn stream_to_file(s: &IStream, out: &Path, total: u64, prog: &Progress) -> Result<(), String> {
    use std::io::Write;
    let tmp = out.with_extension("iso.part");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", out.display()))?);
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        if prog.is_cancelled() {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(crate::engine::export::CANCELLED.into());
        }
        let mut n = 0u32;
        let hr: HRESULT = unsafe { s.Read(buf.as_mut_ptr() as *mut _, buf.len() as u32, Some(&mut n)) };
        if hr.is_err() {
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            return Err(msg(hr.into()));
        }
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n as usize]).map_err(|e| e.to_string())?;
        done += n as u64;
        prog.set(0.1 + 0.9 * (done as f32 / total.max(1) as f32).min(1.0), "Writing the image");
    }
    f.flush().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(())
}

/// Build the image and save it as `out` (`.iso`). Returns its blocks.
pub fn write_iso(volume: &str, items: &[DataItem], size: DiscSize, out: &Path, prog: &Progress) -> Result<u64, String> {
    let _c = com();
    let fs = build_image(volume, items, Limit::Blocks(size.data_blocks()))?;
    unsafe {
        let res = fs.CreateResultImage().map_err(msg)?;
        let blocks = res.TotalBlocks().map_err(msg)?.max(0) as u64;
        super::check_data_capacity(blocks, size)?;
        let s = res.ImageStream().map_err(msg)?;
        stream_to_file(&s, out, blocks * DATA_BLOCK, prog)?;
        Ok(blocks)
    }
}

fn data_format(r: &IDiscRecorder2, opts: &BurnOptions) -> Result<IDiscFormat2Data, String> {
    let f: IDiscFormat2Data = create(&MsftDiscFormat2Data)?;
    unsafe {
        f.SetRecorder(r).map_err(msg)?;
        f.SetClientName(&BSTR::from(CLIENT)).map_err(msg)?;
        f.SetForceMediaToBeClosed(vb(opts.close)).map_err(msg)?;
        f.SetWriteSpeed(if opts.speed > 0 { opts.speed } else { -1 }, VARIANT_FALSE).map_err(msg)?;
        if let Ok(v) = f.cast::<IBurnVerification>() {
            let _ = v.SetBurnVerificationLevel(if opts.verify { IMAPI_BURN_VERIFICATION_QUICK } else { IMAPI_BURN_VERIFICATION_NONE });
        }
    }
    Ok(f)
}

/// Burn a data disc (or, `test_only`, check everything and stop before writing).
pub fn burn_data(id: &str, volume: &str, items: &[DataItem], opts: &BurnOptions, test_only: bool, prog: &Progress) -> Result<String, String> {
    let _c = com();
    let r = recorder(id)?;
    let f = data_format(&r, opts)?;
    unsafe {
        let fs = build_image(volume, items, Limit::Drive(&r))?;
        let res = fs.CreateResultImage().map_err(msg)?;
        let blocks = res.TotalBlocks().map_err(msg)?.max(0) as u64;
        let free = f.FreeSectorsOnMedia().map_err(msg)?.max(0) as u64;
        if blocks > free {
            return Err(format!("Too big for the disc in the drive: {} MB, it has {} MB free.", blocks * DATA_BLOCK / 1_000_000, free * DATA_BLOCK / 1_000_000));
        }
        if test_only {
            return Ok(format!("Test passed: {} MB fits the disc in the drive ({} MB free); nothing was written", blocks * DATA_BLOCK / 1_000_000, free * DATA_BLOCK / 1_000_000));
        }
        let s = res.ImageStream().map_err(msg)?;
        let sink = sink(SinkKind::Data, DDiscFormat2DataEvents::IID, prog);
        let cookie = advise(&f, &DDiscFormat2DataEvents::IID, &sink);
        let w = f.Write(&s).map_err(msg);
        unadvise(cookie);
        w?;
        if opts.eject {
            let _ = r.EjectMedia();
        }
    }
    Ok(format!("Burned {} MB", blocks_mb(items)))
}

fn blocks_mb(items: &[DataItem]) -> u64 {
    items.iter().map(|i| super::tree_bytes(&i.src)).sum::<u64>() / 1_000_000
}

/// Burn converted tracks (raw CD audio, whole sectors) track-at-once. TAO always leaves 2 s between
/// tracks; a longer gap is written as silence at the end of the track before it.
pub fn burn_audio(id: &str, raw: &[std::path::PathBuf], places: &[TrackPlace], opts: &BurnOptions, test_only: bool, prog: &Progress) -> Result<(), String> {
    let _c = com();
    let r = recorder(id)?;
    let tao: IDiscFormat2TrackAtOnce = create(&MsftDiscFormat2TrackAtOnce)?;
    unsafe {
        if !tao.IsCurrentMediaSupported(&r).map(|b| b.as_bool()).unwrap_or(false) {
            return Err("The drive has no blank CD-R / CD-RW for an audio CD.".into());
        }
        tao.SetRecorder(&r).map_err(msg)?;
        tao.SetClientName(&BSTR::from(CLIENT)).map_err(msg)?;
        let free = tao.FreeSectorsOnMedia().map_err(msg)?.max(0) as u64;
        // each TAO track gets its own 150-sector pregap
        let need: u64 = places.iter().map(|p| p.sectors + 150).sum();
        if need > free {
            return Err(format!("Too long for the CD in the drive: {} of music, it has {} free.", super::clock(need as f64 / 75.0), super::clock(free as f64 / 75.0)));
        }
        if test_only {
            return Ok(());
        }
        // gaps longer than TAO's 2 s: pad the previous track
        for (i, p) in places.iter().enumerate().skip(1) {
            let extra = (p.start - p.gap_start).saturating_sub(150);
            if extra > 0 {
                let mut f = std::fs::OpenOptions::new().append(true).open(&raw[i - 1]).map_err(|e| e.to_string())?;
                std::io::Write::write_all(&mut f, &vec![0u8; (extra * AUDIO_SECTOR) as usize]).map_err(|e| e.to_string())?;
            }
        }
        tao.PrepareMedia().map_err(msg)?;
        let _ = tao.SetWriteSpeed(if opts.speed > 0 { opts.speed } else { -1 }, VARIANT_FALSE);
        let sink = sink(SinkKind::Tao, DDiscFormat2TrackAtOnceEvents::IID, prog);
        let cookie = advise(&tao, &DDiscFormat2TrackAtOnceEvents::IID, &sink);
        let mut result = Ok(());
        for (i, path) in raw.iter().enumerate() {
            prog.set(0.5 + 0.48 * i as f32 / raw.len() as f32, format!("Writing track {} of {}", i + 1, raw.len()));
            let st = match file_stream(path) {
                Ok(s) => s,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            };
            if let Err(e) = tao.AddAudioTrack(&st) {
                result = Err(msg(e));
                break;
            }
        }
        unadvise(cookie);
        prog.set(0.99, "Closing the disc");
        let rel = tao.ReleaseMedia().map_err(msg);
        result?;
        rel?;
        if opts.eject {
            let _ = r.EjectMedia();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("se-imapi-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The image target: IMAPI's own file-system image saved as a `.iso`, then its directory read back
    /// with `iso_list` (Joliet names, nested folders, a playlist's songs + `.m3u8`).
    #[test]
    fn data_disc_image_lists_the_files() {
        let d = tmpdir("iso");
        let src = d.join("src");
        std::fs::create_dir_all(src.join("Album/Disc 1")).unwrap();
        std::fs::write(src.join("Album/Disc 1/01 Long name song.mp3"), vec![1u8; 5000]).unwrap();
        std::fs::write(src.join("Album/cover.jpg"), b"jpg").unwrap();
        let single = d.join("readme é.txt");
        std::fs::write(&single, b"hello disc").unwrap();
        let m3u = d.join("Road Trip.m3u8");
        std::fs::write(&m3u, super::super::disc_m3u8(&[("Song".into(), 3.0, "01 Long name song.mp3".into())])).unwrap();
        let items = vec![
            DataItem { src: src.join("Album"), dest: String::new() },
            DataItem { src: single.clone(), dest: String::new() },
            DataItem { src: m3u, dest: "Playlists/Road Trip".into() },
            DataItem { src: src.join("Album/Disc 1/01 Long name song.mp3"), dest: "Playlists/Road Trip".into() },
        ];
        let iso = d.join("out.iso");
        let prog = Progress::new();
        let blocks = match write_iso("My Disc", &items, DiscSize::Cd80, &iso, &prog) {
            Ok(b) => b,
            Err(e) if e.contains("not available") => {
                eprintln!("skipped: {e}");
                return;
            }
            Err(e) => panic!("{e}"),
        };
        assert!(blocks > 0);
        assert_eq!(std::fs::metadata(&iso).unwrap().len(), blocks * DATA_BLOCK);
        let list = super::super::iso_list(&iso).unwrap();
        for want in [
            "Album/",
            "Album/Disc 1/",
            "Album/Disc 1/01 Long name song.mp3",
            "Album/cover.jpg",
            "Playlists/",
            "Playlists/Road Trip/",
            "Playlists/Road Trip/01 Long name song.mp3",
            "Playlists/Road Trip/Road Trip.m3u8",
            "readme é.txt",
        ] {
            assert!(list.iter().any(|x| x == want), "{want} missing from {list:?}");
        }
        assert_eq!(super::super::iso_read(&iso, "readme é.txt").unwrap(), b"hello disc");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_image_too_big_for_the_disc_is_a_clean_error() {
        let d = tmpdir("big");
        let f = d.join("big.bin");
        // sparse 800 MB file: bigger than a CD, no real disk use
        let file = std::fs::File::create(&f).unwrap();
        file.set_len(800_000_000).unwrap();
        drop(file);
        let items = vec![DataItem { src: f, dest: String::new() }];
        let e = super::super::run(
            &super::super::BurnContent::Data { volume: "x".into(), items },
            &super::super::BurnTarget::Image(d.join("x.iso")),
            &BurnOptions { size: DiscSize::Cd80, ..Default::default() },
            &Progress::new(),
        )
        .unwrap_err();
        assert!(e.starts_with("Too big"), "{e}");
        assert!(!d.join("x.iso").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn listing_drives_never_fails_without_one() {
        // the machine may have no burner: an empty list, not an error
        let _ = drives().unwrap_or_default();
    }

    #[test]
    fn volume_labels_are_short() {
        assert_eq!(volume_label("Summer Mix 2024: the best!"), "Summer Mix 2024");
        assert_eq!(volume_label("???"), "DISC");
    }
}
