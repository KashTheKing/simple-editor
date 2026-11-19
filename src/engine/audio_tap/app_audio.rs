//! One application's sound: Windows process loopback (`AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`,
//! Windows 10 2004+), the target process and its children, nothing else - so an "App audio" track of a
//! Roblox recording has the game and not Discord or music.
//!
//! Activation is asynchronous (`ActivateAudioInterfaceAsync` + a completion handler). Like
//! `engine::disc::imapi`'s event sink, the handler is a hand-made COM object (the `#[implement]` macro
//! would need `windows-core` as a direct dependency); it is a static with no state but an event, so its
//! reference count never matters, and activations are serialised by a lock.
//! The stream is 48 kHz stereo f32 (the audio engine converts), written raw like the other taps, with
//! gaps filled from the packets' QPC positions.

use super::{gap_frames, TapState};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::{Interface, GUID, HRESULT};

const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;

type Raw = *mut core::ffi::c_void;

#[repr(C)]
struct HandlerVtbl {
    query: unsafe extern "system" fn(Raw, *const GUID, *mut Raw) -> HRESULT,
    add_ref: unsafe extern "system" fn(Raw) -> u32,
    release: unsafe extern "system" fn(Raw) -> u32,
    completed: unsafe extern "system" fn(Raw, Raw) -> HRESULT,
}

#[repr(C)]
struct Handler {
    vtbl: *const HandlerVtbl,
}
unsafe impl Sync for Handler {}

static VTBL: HandlerVtbl = HandlerVtbl { query: h_query, add_ref: h_ref, release: h_ref, completed: h_completed };
static HANDLER: Handler = Handler { vtbl: &VTBL };
/// The completion event (a Win32 event handle as usize) and the activation lock.
static DONE: Mutex<usize> = Mutex::new(0);

const IID_AGILE: GUID = GUID::from_u128(0x94ea2b94_e9cc_49e0_c0ff_ee64ca8f5b90);

unsafe extern "system" fn h_query(this: Raw, iid: *const GUID, out: *mut Raw) -> HRESULT {
    let iid = unsafe { *iid };
    use windows::Win32::Media::Audio::IActivateAudioInterfaceCompletionHandler;
    if iid == windows::core::IUnknown::IID || iid == IActivateAudioInterfaceCompletionHandler::IID || iid == IID_AGILE {
        unsafe { *out = this };
        HRESULT(0)
    } else {
        unsafe { *out = std::ptr::null_mut() };
        HRESULT(0x8000_4002_u32 as i32) // E_NOINTERFACE
    }
}
unsafe extern "system" fn h_ref(_: Raw) -> u32 {
    1 // static: never freed
}
unsafe extern "system" fn h_completed(_: Raw, _op: Raw) -> HRESULT {
    let ev = *DONE.lock().unwrap_or_else(|e| e.into_inner());
    if ev != 0 {
        unsafe {
            let _ = windows::Win32::System::Threading::SetEvent(windows::Win32::Foundation::HANDLE(ev as *mut _));
        }
    }
    HRESULT(0)
}

/// An IAudioClient capturing `pid`'s sound.
fn activate(pid: u32) -> Result<windows::Win32::Media::Audio::IAudioClient, String> {
    use std::mem::ManuallyDrop;
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::BLOB;
    use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
    use windows::Win32::System::Variant::VT_BLOB;
    let mut guard = DONE.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        if *guard == 0 {
            let ev = CreateEventW(None, true, false, None).map_err(|e| e.message())?;
            *guard = ev.0 as usize;
        }
        let ev = HANDLE(*guard as *mut _);
        let _ = ResetEvent(ev);
        drop(guard); // the completion runs on another thread and takes the lock to read the event
        let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };
        // never dropped: the windows crate's `Drop for PROPVARIANT` calls PropVariantClear, which would
        // CoTaskMemFree the blob - here a pointer to `params` on the stack (that corrupted the heap)
        let mut pv = ManuallyDrop::new(PROPVARIANT::default());
        {
            let inner = &mut *pv.Anonymous.Anonymous;
            inner.vt = VT_BLOB;
            inner.Anonymous.blob =
                BLOB { cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, pBlobData: &mut params as *mut _ as *mut u8 };
        }
        let handler = ManuallyDrop::new(IActivateAudioInterfaceCompletionHandler::from_raw(&HANDLER as *const Handler as Raw));
        let op = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*pv), &*handler)
            .map_err(|e| format!("App audio needs Windows 10 2004 or later ({})", e.message()))?;
        if WaitForSingleObject(ev, 5000) != WAIT_OBJECT_0 {
            return Err("App audio didn't start.".into());
        }
        let mut hr = HRESULT(0);
        let mut unk = None;
        op.GetActivateResult(&mut hr, &mut unk).map_err(|e| e.message())?;
        hr.ok().map_err(|e| format!("App audio: {}", e.message()))?;
        unk.ok_or("App audio: no client")?.cast::<IAudioClient>().map_err(|e| e.message())
    }
}

pub(super) fn run(pid: u32, out: Option<PathBuf>, st: &TapState, stop: &Receiver<()>) -> Result<(), String> {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let client = activate(pid)?;
        let wfx = WAVEFORMATEX {
            wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
            nChannels: CHANNELS as u16,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * CHANNELS * 4,
            nBlockAlign: (CHANNELS * 4) as u16,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM;
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &wfx, None)
            .map_err(|e| format!("App audio: {}", e.message()))?;
        let ev = CreateEventW(None, false, false, None).map_err(|e| e.message())?;
        client.SetEventHandle(ev).map_err(|e| e.message())?;
        let cap: IAudioCaptureClient = client.GetService().map_err(|e| e.message())?;
        st.rate.store(RATE, Ordering::SeqCst);
        st.channels.store(CHANNELS, Ordering::SeqCst);
        client.Start().map_err(|e| e.message())?;
        let mut file = match &out {
            Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?)),
            None => None,
        };
        let mut first_qpc: Option<u64> = None;
        let mut frames: u64 = 0;
        let mut peak = 0f32;
        let mut last_flush = std::time::Instant::now();
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match stop.try_recv() {
                Err(TryRecvError::Empty) => {}
                _ => break,
            }
            let _ = WaitForSingleObject(ev, 100);
            loop {
                let n = cap.GetNextPacketSize().unwrap_or(0);
                if n == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let (mut count, mut fl, mut dev, mut qpc) = (0u32, 0u32, 0u64, 0u64);
                if cap.GetBuffer(&mut data, &mut count, &mut fl, Some(&mut dev), Some(&mut qpc)).is_err() {
                    break;
                }
                // QPC positions are in 100 ns units
                let first = *first_qpc.get_or_insert_with(|| {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                    st.t0_us.store(now.as_micros().max(1) as u64, Ordering::SeqCst);
                    qpc
                });
                let at = qpc.saturating_sub(first) as f64 / 1e7;
                let gap = gap_frames(at, RATE, frames);
                bytes.clear();
                bytes.resize((gap * CHANNELS as u64 * 4) as usize, 0);
                let n_samples = (count * CHANNELS) as usize;
                if fl & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    bytes.resize(bytes.len() + n_samples * 4, 0);
                } else {
                    let samples = std::slice::from_raw_parts(data as *const f32, n_samples);
                    for &v in samples {
                        peak = peak.max(v.abs());
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                }
                frames += gap + count as u64;
                let _ = cap.ReleaseBuffer(count);
                if let Some(f) = file.as_mut() {
                    f.write_all(&bytes).map_err(|e| format!("Writing audio: {e}"))?;
                }
            }
            if last_flush.elapsed() >= Duration::from_millis(100) {
                st.peak.store(peak.min(1.0).to_bits(), Ordering::Relaxed);
                peak = 0.0;
                if let Some(f) = file.as_mut() {
                    f.flush().map_err(|e| format!("Writing audio: {e}"))?;
                }
                last_flush = std::time::Instant::now();
            }
        }
        let _ = client.Stop();
        if let Some(f) = file.as_mut() {
            let _ = f.flush();
        }
        let _ = windows::Win32::Foundation::CloseHandle(ev);
    }
    Ok(())
}
