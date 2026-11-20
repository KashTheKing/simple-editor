//! Shortcuts (IShellLinkW through hand-written COM vtables: windows-sys has no interfaces), Explorer
//! notifications, file versions, running processes, the Credential Manager and known folders.

use super::wide;
use crate::setup::Shell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::core::GUID;

pub struct WinShell;

type HRESULT = i32;

#[repr(C)]
struct Unknown {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// IShellLinkW, in vtable order (only the slots we call are typed).
#[repr(C)]
struct ShellLinkVtbl {
    base: Unknown,
    get_path: usize,
    get_id_list: usize,
    set_id_list: usize,
    get_description: usize,
    set_description: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    get_working_directory: usize,
    set_working_directory: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    get_arguments: usize,
    set_arguments: usize,
    get_hotkey: usize,
    set_hotkey: usize,
    get_show_cmd: usize,
    set_show_cmd: usize,
    get_icon_location: usize,
    set_icon_location: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
    set_relative_path: usize,
    resolve: usize,
    set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
}

/// IPersistFile
#[repr(C)]
struct PersistFileVtbl {
    base: Unknown,
    get_class_id: usize,
    is_dirty: usize,
    load: usize,
    save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
}

#[repr(C)]
struct PropertyKey {
    fmtid: GUID,
    pid: u32,
}

/// PROPVARIANT holding a VT_LPWSTR (24 bytes on x64, 16 on x86).
#[repr(C)]
struct PropVariant {
    vt: u16,
    r1: u16,
    r2: u16,
    r3: u16,
    ptr: *const u16,
    pad: usize,
}

/// IPropertyStore
#[repr(C)]
struct PropertyStoreVtbl {
    base: Unknown,
    get_count: usize,
    get_at: usize,
    get_value: usize,
    set_value: unsafe extern "system" fn(*mut c_void, *const PropertyKey, *const PropVariant) -> HRESULT,
    commit: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
const IID_SHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_PERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
const IID_PROPERTY_STORE: GUID = GUID::from_u128(0x886d8eeb_8cf2_4446_8d02_cdba1dbdcf99);
/// PKEY_AppUserModel_ID
const PKEY_APP_USER_MODEL_ID: PropertyKey = PropertyKey { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

/// A COM pointer released on drop.
struct Com(*mut c_void);
impl Com {
    unsafe fn vtbl<T>(&self) -> &T {
        &**(self.0 as *mut *const T)
    }
    fn cast(&self, iid: &GUID) -> Option<Com> {
        let mut out = null_mut();
        let hr = unsafe { (self.vtbl::<Unknown>().query_interface)(self.0, iid, &mut out) };
        (hr >= 0 && !out.is_null()).then_some(Com(out))
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { (self.vtbl::<Unknown>().release)(self.0) };
    }
}

fn check(hr: HRESULT, what: &str) -> Result<(), String> {
    if hr >= 0 {
        Ok(())
    } else {
        Err(format!("{what} failed (0x{:08x})", hr as u32))
    }
}

impl Shell for WinShell {
    fn shortcut(&self, lnk: &Path, target: &Path, desc: &str) -> Result<(), String> {
        use windows_sys::Win32::System::Com::*;
        if let Some(d) = lnk.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        unsafe {
            // S_FALSE (already initialised on this thread) is fine
            CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
            let mut p = null_mut();
            check(CoCreateInstance(&CLSID_SHELL_LINK, null_mut(), CLSCTX_INPROC_SERVER, &IID_SHELL_LINK_W, &mut p), "creating a shortcut")?;
            let link = Com(p);
            let v = link.vtbl::<ShellLinkVtbl>();
            let t = wide(&target.display().to_string());
            check((v.set_path)(link.0, t.as_ptr()), "the shortcut's target")?;
            if let Some(dir) = target.parent() {
                let d = wide(&dir.display().to_string());
                (v.set_working_directory)(link.0, d.as_ptr());
            }
            let de = wide(desc);
            (v.set_description)(link.0, de.as_ptr());
            (v.set_icon_location)(link.0, t.as_ptr(), 0);
            // the taskbar identity the app's window uses, so a pinned shortcut and the window group together
            if let Some(ps) = link.cast(&IID_PROPERTY_STORE) {
                let id = wide(crate::shell_keys::APP_USER_MODEL_ID);
                let pv = PropVariant { vt: 31 /* VT_LPWSTR */, r1: 0, r2: 0, r3: 0, ptr: id.as_ptr(), pad: 0 };
                let psv = ps.vtbl::<PropertyStoreVtbl>();
                if (psv.set_value)(ps.0, &PKEY_APP_USER_MODEL_ID, &pv) >= 0 {
                    (psv.commit)(ps.0);
                }
            }
            let pf = link.cast(&IID_PERSIST_FILE).ok_or("saving the shortcut is not supported")?;
            let path = wide(&lnk.display().to_string());
            check((pf.vtbl::<PersistFileVtbl>().save)(pf.0, path.as_ptr(), 1), "saving the shortcut")?;
        }
        Ok(())
    }

    fn notify_assoc(&self) {
        use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
    }

    fn file_version(&self, exe: &Path) -> Option<String> {
        use windows_sys::Win32::Storage::FileSystem::*;
        let w = wide(&exe.display().to_string());
        unsafe {
            let n = GetFileVersionInfoSizeW(w.as_ptr(), null_mut());
            if n == 0 {
                return None;
            }
            let mut buf = vec![0u8; n as usize];
            if GetFileVersionInfoW(w.as_ptr(), 0, n, buf.as_mut_ptr() as *mut c_void) == 0 {
                return None;
            }
            let mut info: *mut c_void = null_mut();
            let mut len = 0u32;
            let root = wide("\\");
            if VerQueryValueW(buf.as_ptr() as *const c_void, root.as_ptr(), &mut info, &mut len) == 0 || info.is_null() {
                return None;
            }
            let f = &*(info as *const VS_FIXEDFILEINFO);
            let (ms, ls) = (f.dwProductVersionMS, f.dwProductVersionLS);
            Some(format!("{}.{}.{}", ms >> 16, ms & 0xFFFF, ls >> 16))
        }
    }

    fn running(&self, exe: &Path) -> bool {
        !processes_of(exe).is_empty()
    }

    fn delete_credential(&self, target: &str) {
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        let t = wide(target);
        unsafe { CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) };
    }
}

/// Process ids running `exe` (compared by full path, case-insensitively).
pub fn processes_of(exe: &Path) -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    use windows_sys::Win32::System::Threading::*;
    let want = exe.display().to_string().to_lowercase();
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let name = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            if String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase() == name {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, e.th32ProcessID);
                if !h.is_null() {
                    let mut buf = [0u16; 1024];
                    let mut n = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut n) != 0
                        && String::from_utf16_lossy(&buf[..n as usize]).to_lowercase() == want
                    {
                        out.push(e.th32ProcessID);
                    }
                    CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    out
}

/// The Desktop folder (moved or redirected).
pub fn desktop() -> Option<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, SHGetKnownFolderPath};
    unsafe {
        let mut p: windows_sys::core::PWSTR = null_mut();
        if SHGetKnownFolderPath(&FOLDERID_Desktop, 0, null_mut(), &mut p) < 0 || p.is_null() {
            return None;
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        CoTaskMemFree(p as *const c_void);
        Some(PathBuf::from(s))
    }
}

/// Start a program without a console window.
pub fn spawn_hidden(exe: &Path, args: &[&str]) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(exe).args(args).creation_flags(0x0800_0000).spawn().map(|_| ())
}

/// Delete `file` a moment after this process exits (the temporary uninstaller removes itself).
pub fn delete_after_exit(file: &Path) {
    use std::os::windows::process::CommandExt;
    let cmd = std::env::var_os("ComSpec").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
    let _ = std::process::Command::new(cmd)
        .raw_arg(format!("/d /c ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"", file.display()))
        .creation_flags(0x0800_0000)
        .spawn();
}
