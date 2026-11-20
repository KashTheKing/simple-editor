//! ws:parity-io: choosing the GPU on a machine with more than one (a laptop's integrated + discrete pair, or
//! a desktop with two cards).
//!
//! Two levers, both read once at start-up, so a change applies after a restart:
//!  * Windows' own per-app preference (`HKCU\Software\Microsoft\DirectX\UserGpuPreferences`, the value
//!    Settings ▸ System ▸ Display ▸ Graphics writes for an app): it decides which GPU the OpenGL window and
//!    a default D3D11 device land on. "Power saving" = 1, "High performance" = 2; Automatic removes it.
//!  * The hardware-decode device (`media::mf`): with a named adapter it is created on exactly that one.
//!    Zero-copy frames need it on the same GPU as the window, so naming one also writes the Windows
//!    preference that puts the window there.
//!
//! `Settings.gpu_adapter`: "" = Automatic, "power", "performance", or an adapter's name.

use std::sync::{OnceLock, RwLock};

/// One adapter as DXGI lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Adapter {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    /// Dedicated video memory in MB.
    pub vram_mb: u64,
    /// Windows' order for "High performance": 0 = the one it would pick.
    pub perf_rank: usize,
    /// Windows' order for "Power saving".
    pub power_rank: usize,
}

/// What `Settings.gpu_adapter` asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    PowerSaving,
    HighPerformance,
    Named(String),
}

impl Choice {
    pub fn parse(s: &str) -> Choice {
        match s.trim() {
            "" | "auto" => Choice::Auto,
            "power" => Choice::PowerSaving,
            "performance" => Choice::HighPerformance,
            name => Choice::Named(name.to_string()),
        }
    }
    pub fn setting(&self) -> String {
        match self {
            Choice::Auto => String::new(),
            Choice::PowerSaving => "power".into(),
            Choice::HighPerformance => "performance".into(),
            Choice::Named(n) => n.clone(),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Choice::Auto => "Automatic".into(),
            Choice::PowerSaving => "Power saving".into(),
            Choice::HighPerformance => "High performance".into(),
            Choice::Named(n) => n.clone(),
        }
    }
    /// The Windows preference value (0 = none / let Windows decide) for this choice among `list`.
    pub fn windows_pref(&self, list: &[Adapter]) -> u32 {
        match self {
            Choice::Auto => 0,
            Choice::PowerSaving => 1,
            Choice::HighPerformance => 2,
            Choice::Named(n) => match list.iter().find(|a| a.name == *n) {
                Some(a) if a.perf_rank == 0 => 2,
                Some(a) if a.power_rank == 0 => 1,
                _ => 0,
            },
        }
    }
}

/// The registry data for a preference (None = remove our value).
pub fn pref_data(pref: u32) -> Option<String> {
    (pref != 0).then(|| format!("GpuPreference={pref};"))
}

/// The hardware adapters, in DXGI order (no software rasteriser). Cached: the list only changes with
/// hardware, and the choice applies at the next start anyway.
pub fn adapters() -> &'static [Adapter] {
    static LIST: OnceLock<Vec<Adapter>> = OnceLock::new();
    LIST.get_or_init(enumerate)
}

fn enumerate() -> Vec<Adapter> {
    use windows::Win32::Graphics::Dxgi::*;
    let mut out: Vec<Adapter> = Vec::new();
    // SAFETY: plain DXGI enumeration; every interface is released when it drops.
    unsafe {
        let Ok(f) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let rank = |pref: DXGI_GPU_PREFERENCE| -> Vec<(u32, u32)> {
            let Ok(f6) = windows::core::Interface::cast::<IDXGIFactory6>(&f) else { return Vec::new() };
            (0..8u32)
                .map_while(|i| f6.EnumAdapterByGpuPreference::<IDXGIAdapter1>(i, pref).ok())
                .filter_map(|a| a.GetDesc1().ok())
                .map(|d| (d.VendorId, d.DeviceId))
                .collect()
        };
        let (perf, power) = (rank(DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE), rank(DXGI_GPU_PREFERENCE_MINIMUM_POWER));
        for i in 0..8u32 {
            let Ok(a) = f.EnumAdapters1(i) else { break };
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let len = d.Description.iter().position(|c| *c == 0).unwrap_or(d.Description.len());
            let name = String::from_utf16_lossy(&d.Description[..len]).trim().to_string();
            let id = (d.VendorId, d.DeviceId);
            let pos = |l: &[(u32, u32)]| l.iter().position(|x| *x == id).unwrap_or(usize::MAX);
            if out.iter().any(|o| o.name == name) {
                continue; // the same card on two outputs
            }
            out.push(Adapter {
                name,
                vendor: d.VendorId,
                device: d.DeviceId,
                vram_mb: d.DedicatedVideoMemory as u64 >> 20,
                perf_rank: pos(&perf),
                power_rank: pos(&power),
            });
        }
    }
    out
}

static CHOICE: RwLock<Option<Choice>> = RwLock::new(None);

/// The choice the hardware-decode device follows (`App::build`, before any decoder opens).
pub fn set_choice(s: &str) {
    *CHOICE.write().unwrap_or_else(|e| e.into_inner()) = Some(Choice::parse(s));
}

/// The adapter the decode device should be created on: the named one, the first by Windows' order for
/// Power saving / High performance, or None for the system default.
pub fn decode_adapter() -> Option<windows::Win32::Graphics::Dxgi::IDXGIAdapter> {
    use windows::Win32::Graphics::Dxgi::*;
    let choice = CHOICE.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or(Choice::Auto);
    let pref = match &choice {
        Choice::Auto => return None,
        Choice::PowerSaving => DXGI_GPU_PREFERENCE_MINIMUM_POWER,
        Choice::HighPerformance => DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
        Choice::Named(_) => DXGI_GPU_PREFERENCE_UNSPECIFIED,
    };
    // SAFETY: as in `enumerate`.
    unsafe {
        let f6: IDXGIFactory6 = CreateDXGIFactory1::<IDXGIFactory6>().ok()?;
        for i in 0..8u32 {
            let a: IDXGIAdapter1 = f6.EnumAdapterByGpuPreference(i, pref).ok()?;
            let d = a.GetDesc1().ok()?;
            let len = d.Description.iter().position(|c| *c == 0).unwrap_or(d.Description.len());
            let name = String::from_utf16_lossy(&d.Description[..len]).trim().to_string();
            let hit = match &choice {
                Choice::Named(n) => *n == name,
                _ => d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0,
            };
            if hit {
                return windows::core::Interface::cast(&a).ok();
            }
        }
    }
    None
}

const PREFS_KEY: &str = r"Software\Microsoft\DirectX\UserGpuPreferences";

/// Write (or remove) Windows' per-app preference for this exe. Called when the setting changes.
pub fn apply_windows_pref(choice: &Choice) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_string_lossy().into_owned();
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(PREFS_KEY).map_err(|e| e.to_string())?;
    match pref_data(choice.windows_pref(adapters())) {
        Some(data) => key.set_value(&exe, &data).map_err(|e| e.to_string()),
        None => match key.delete_value(&exe) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> Vec<Adapter> {
        vec![
            Adapter { name: "Intel(R) UHD Graphics".into(), vendor: 0x8086, device: 1, vram_mb: 128, perf_rank: 1, power_rank: 0 },
            Adapter { name: "NVIDIA GeForce RTX 3060 Laptop GPU".into(), vendor: 0x10DE, device: 2, vram_mb: 6144, perf_rank: 0, power_rank: 1 },
        ]
    }

    #[test]
    fn choices_round_trip_through_the_setting() {
        for c in [Choice::Auto, Choice::PowerSaving, Choice::HighPerformance, Choice::Named("X".into())] {
            assert_eq!(Choice::parse(&c.setting()), c);
        }
        assert_eq!(Choice::parse("auto"), Choice::Auto);
        assert_eq!(Choice::Auto.label(), "Automatic");
    }

    #[test]
    fn a_named_adapter_maps_to_the_windows_preference_that_selects_it() {
        let l = pair();
        assert_eq!(Choice::Named("NVIDIA GeForce RTX 3060 Laptop GPU".into()).windows_pref(&l), 2);
        assert_eq!(Choice::Named("Intel(R) UHD Graphics".into()).windows_pref(&l), 1);
        assert_eq!(Choice::Named("Unplugged card".into()).windows_pref(&l), 0);
        assert_eq!((Choice::Auto.windows_pref(&l), Choice::HighPerformance.windows_pref(&l)), (0, 2));
        assert_eq!(pref_data(2).as_deref(), Some("GpuPreference=2;"));
        assert_eq!(pref_data(0), None);
    }
}
