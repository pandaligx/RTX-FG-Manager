//! Narrow binding to the public NVIDIA DRS ABI. IDs, versions and layouts are from
//! NVIDIA/nvapi 51e5aae4 (nvapi_interface.h, nvapi.h and NvApiDriverSettings.h).
use super::{Settings, State};
use anyhow::{Context, Result, ensure};
use libloading::os::windows::Library;
use std::{ffi::c_void, mem::size_of, ptr};
use windows::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32;

type Handle = *mut c_void;
type Status = i32;
type Simple = unsafe extern "C" fn() -> Status;
type Create = unsafe extern "C" fn(*mut Handle) -> Status;
type OnSession = unsafe extern "C" fn(Handle) -> Status;
type GetProfile = unsafe extern "C" fn(Handle, *mut Handle) -> Status;
type Find = unsafe extern "C" fn(Handle, *const u16, *mut Handle, *mut Application) -> Status;
type Info = unsafe extern "C" fn(Handle, Handle, *mut Profile) -> Status;
type Apps = unsafe extern "C" fn(Handle, Handle, u32, *mut u32, *mut Application) -> Status;
type Read = unsafe extern "C" fn(Handle, Handle, u32, *mut Setting) -> Status;
type Set = unsafe extern "C" fn(Handle, Handle, *mut Setting) -> Status;
type Delete = unsafe extern "C" fn(Handle, Handle, u32) -> Status;

// NVDRS settings contain two unions. Their largest variant is a DWORD length
// followed by 4096 bytes, packed to four-byte alignment. Only DWORDs are used.
#[repr(C)]
struct Setting {
    version: u32,
    name: [u16; 2048],
    id: u32,
    kind: u32,
    location: u32,
    predefined: u32,
    has_default: u32,
    default: [u32; 1025],
    current: [u32; 1025],
}
#[repr(C)]
struct Profile {
    version: u32,
    name: [u16; 2048],
    gpu_support: u32,
    predefined: u32,
    apps: u32,
    settings: u32,
}
#[repr(C)]
#[derive(Clone)]
struct Application {
    version: u32,
    predefined: u32,
    name: [u16; 2048],
    friendly: [u16; 2048],
    launcher: [u16; 2048],
    folder: [u16; 2048],
    flags: u32,
    command_line: [u16; 2048],
}
macro_rules! versioned {
    ($ty:ty, $size:literal, $revision:literal) => {{
        const {
            assert!(size_of::<$ty>() == $size);
        }
        // SAFETY: these SDK buffers contain only integer fields and arrays.
        let mut v: $ty = unsafe { std::mem::zeroed() };
        v.version = $size | ($revision << 16);
        v
    }};
}
const GENERATED_FRAMES: u32 = 0x104d6667; // NGX_DLSSG_MULTI_FRAME_COUNT_ID
fn ok(code: Status) -> Result<()> {
    ensure!(
        code == 0,
        "无法访问《迈尔斯》的 NVIDIA 配置（NVAPI {code}）"
    );
    Ok(())
}
struct Session {
    _library: Library,
    session: Handle,
    game: Handle,
    destroy: OnSession,
    unload: Simple,
    save: OnSession,
    read: Read,
    set: Set,
    delete: Delete,
}
impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: open initialized the library and owns this session, if created.
        unsafe {
            if !self.session.is_null() {
                (self.destroy)(self.session);
            }
            (self.unload)();
        }
    }
}
impl Session {
    fn open() -> Result<Self> {
        // SAFETY: the loader is restricted to the Windows system directory. The
        // exported query and the typed IDs below follow NVIDIA's public x64 ABI.
        let lib =
            unsafe { Library::load_with_flags("nvapi64.dll", LOAD_LIBRARY_SEARCH_SYSTEM32.0) }
                .context("无法加载 NVIDIA 驱动接口")?;
        type Query = unsafe extern "C" fn(u32) -> *const c_void;
        // SAFETY: the SDK declares this exact signature for its query export.
        let query: Query = unsafe { *lib.get(b"nvapi_QueryInterface\0")? };
        macro_rules! entry {
            ($id:literal, $signature:ty) => {{
                // SAFETY: the numeric ID names a public SDK entry with this signature.
                let address = unsafe { query($id) };
                ensure!(!address.is_null(), "NVIDIA 驱动缺少游戏配置接口");
                // SAFETY: the address belongs to the library retained for the session.
                unsafe { std::mem::transmute::<*const c_void, $signature>(address) }
            }};
        }
        let init = entry!(0x0150e828, Simple);
        let unload = entry!(0xd22bdd7e, Simple);
        let create = entry!(0x0694d52e, Create);
        let find = entry!(0xeee566b2, Find);
        let base = entry!(0xda8466a0, GetProfile);
        let global = entry!(0x617bff9f, GetProfile);
        let info = entry!(0x61cd6fd6, Info);
        let apps = entry!(0x7fa2173a, Apps);
        let destroy = entry!(0xdad9cff8, OnSession);
        let load = entry!(0x375dbd6b, OnSession);
        let save = entry!(0xfcbc7e14, OnSession);
        let read = entry!(0x73bf8338, Read);
        let set = entry!(0x577dd202, Set);
        let delete = entry!(0xe4a26362, Delete);
        // SAFETY: no parameters, the system library is loaded.
        ok(unsafe { init() })?;
        let mut s = Self {
            _library: lib,
            session: ptr::null_mut(),
            game: ptr::null_mut(),
            destroy,
            unload,
            save,
            read,
            set,
            delete,
        };
        let mut name = [0u16; 2048];
        for (slot, ch) in name.iter_mut().zip("MilesMorales.exe".encode_utf16()) {
            *slot = ch;
        }
        let mut app = versioned!(Application, 20492, 4);
        let mut profile = versioned!(Profile, 4116, 1);
        let (mut base_profile, mut global_profile) = (ptr::null_mut(), ptr::null_mut());
        // SAFETY: output storage is correctly sized and remains live throughout.
        // Each subsequent call is reached only after the previous handle is valid.
        unsafe {
            ok(create(&mut s.session))?;
            ok(load(s.session))?;
            ok(find(s.session, name.as_ptr(), &mut s.game, &mut app))?;
            ok(base(s.session, &mut base_profile))?;
            ok(global(s.session, &mut global_profile))?;
            ok(info(s.session, s.game, &mut profile))?;
        }
        ensure!(
            !s.game.is_null()
                && s.game != base_profile
                && s.game != global_profile
                && profile.predefined != 0
                && (1..=16).contains(&profile.apps)
                && crate::win::from_wide(&profile.name) == "Marvel's Spider-Man: Miles Morales",
            "未找到独立的《迈尔斯》NVIDIA 配置"
        );
        let mut count = profile.apps;
        let mut list = vec![versioned!(Application, 20492, 4); count as usize];
        // SAFETY: list holds count initialized NVDRS_APPLICATION_V4 buffers.
        ok(unsafe { apps(s.session, s.game, 0, &mut count, list.as_mut_ptr()) })?;
        let leaves = list
            .iter()
            .map(|app| {
                let name = crate::win::from_wide(&app.name);
                std::path::Path::new(&name)
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
            })
            .collect::<Vec<_>>();
        ensure!(
            count == profile.apps
                && leaves.iter().all(|leaf| matches!(
                    leaf.as_str(),
                    "milesmorales.exe" | "i31_steam_release.exe" | "i31_steam_finaldm.exe"
                ))
                && leaves.iter().any(|v| v == "milesmorales.exe"),
            "此 NVIDIA 配置还关联了其他程序，已保留原设置"
        );
        Ok(s)
    }
    fn values(&self) -> Result<State> {
        let mut s = versioned!(Setting, 12320, 1);
        // SAFETY: valid game/session and versioned writable setting.
        let code = unsafe { (self.read)(self.session, self.game, GENERATED_FRAMES, &mut s) };
        if code == -160 {
            return Ok(None);
        } // NVAPI_SETTING_NOT_FOUND
        ok(code)?;
        ensure!(s.kind == 0, "NVIDIA 配置类型不兼容");
        Ok((s.location == 0 && s.predefined == 0).then_some(s.current[0]))
    }
}

pub(super) struct Driver;
impl Settings for Driver {
    fn read(&mut self) -> Result<State> {
        Session::open()?.values()
    }
    fn replace(&mut self, expected: State, next: State) -> Result<()> {
        let s = Session::open()?;
        ensure!(s.values()? == expected, "NVIDIA 配置已变化，请重试");
        if expected != next {
            if let Some(value) = next {
                let mut setting = versioned!(Setting, 12320, 1);
                setting.id = GENERATED_FRAMES;
                setting.current[0] = value;
                // SAFETY: only the public count DWORD on the verified game profile is written.
                ok(unsafe { (s.set)(s.session, s.game, &mut setting) })?;
            } else {
                // SAFETY: removes only the per-game override, exposing the driver's default.
                ok(unsafe { (s.delete)(s.session, s.game, GENERATED_FRAMES) })?;
            }
        }
        // SAFETY: the change is staged in a validated DRS session.
        ok(unsafe { (s.save)(s.session) })?;
        drop(s);
        ensure!(self.read()? == next, "NVIDIA 配置未写入，请重试");
        Ok(())
    }
}
