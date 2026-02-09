#![cfg(target_os = "windows")]

use std::{os::windows::ffi::OsStringExt, path::PathBuf};
use windows::{
    Win32::{
        Foundation::{
            CLASS_E_CLASSNOTAVAILABLE, E_FAIL, E_INVALIDARG, E_NOTIMPL, ERROR_INSUFFICIENT_BUFFER,
            GetLastError, HINSTANCE, MAX_PATH,
        },
        Globalization::u_strlen,
        System::{
            Com::{IBindCtx, IClassFactory, IClassFactory_Impl},
            LibraryLoader::GetModuleFileNameW,
            SystemServices::DLL_PROCESS_ATTACH,
        },
        UI::Shell::{
            ECF_DEFAULT, ECS_ENABLED, IEnumExplorerCommand, IExplorerCommand,
            IExplorerCommand_Impl, IShellItemArray, SHStrDupW, SIGDN_FILESYSPATH,
        },
    },
    core::{BOOL, GUID, HRESULT, HSTRING, Ref, Result, implement},
};

static mut DLL_INSTANCE: HINSTANCE = HINSTANCE(std::ptr::null_mut());

#[unsafe(no_mangle)]
extern "system" fn DllMain(hinstdll: HINSTANCE, fdwreason: u32, _: *mut core::ffi::c_void) -> bool {
    if fdwreason == DLL_PROCESS_ATTACH {
        unsafe { DLL_INSTANCE = hinstdll };
    }
    true
}

#[implement(IExplorerCommand)]
struct ExplorerCommand;

#[allow(non_snake_case)]
impl IExplorerCommand_Impl for ExplorerCommand_Impl {
    fn GetTitle(&self, _: Ref<IShellItemArray>) -> Result<windows_core::PWSTR> {
        unsafe { SHStrDupW(&HSTRING::from("Open with Cunning3D")) }
    }
    fn GetIcon(&self, _: Ref<IShellItemArray>) -> Result<windows_core::PWSTR> {
        let Some(exe) = get_cunning3d_exe_path() else { return Err(E_FAIL.into()); };
        unsafe { SHStrDupW(&HSTRING::from(exe)) }
    }
    fn GetToolTip(&self, _: Ref<IShellItemArray>) -> Result<windows_core::PWSTR> { Err(E_NOTIMPL.into()) }
    fn GetCanonicalName(&self) -> Result<windows_core::GUID> { Ok(GUID::zeroed()) }
    fn GetState(&self, _: Ref<IShellItemArray>, _: BOOL) -> Result<u32> { Ok(ECS_ENABLED.0 as _) }

    fn Invoke(&self, psiitemarray: Ref<IShellItemArray>, _: Ref<IBindCtx>) -> Result<()> {
        let items = psiitemarray.ok()?;
        let Some(exe) = get_cunning3d_exe_path() else { return Ok(()); };
        let count = unsafe { items.GetCount()? };
        for idx in 0..count {
            let item = unsafe { items.GetItemAt(idx)? };
            let p = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)?.to_string()? };
            #[allow(clippy::disallowed_methods, reason = "Explorer callback; no async context.")]
            std::process::Command::new(&exe).arg(&p).spawn().map_err(|_| E_INVALIDARG)?;
        }
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> { Ok(ECF_DEFAULT.0 as _) }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> { Err(E_NOTIMPL.into()) }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<windows_core::IUnknown>,
        riid: *const windows_core::GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        unsafe { *ppvobject = std::ptr::null_mut() };
        if punkouter.is_some() {
            return Err(E_INVALIDARG.into());
        }
        let cmd: IExplorerCommand = ExplorerCommand {}.into();
        let r = unsafe { cmd.query(riid, ppvobject).ok() };
        if r.is_ok() {
            unsafe { *ppvobject = cmd.into_raw() };
        }
        r
    }
    fn LockServer(&self, _: BOOL) -> Result<()> { Ok(()) }
}

#[cfg(all(feature = "stable", not(feature = "preview")))]
const MODULE_ID: GUID = GUID::from_u128(0xb7d2d1a5_4f8c_4db4_8b1f_2a1fd45b3f3a);
#[cfg(all(feature = "preview", not(feature = "stable")))]
const MODULE_ID: GUID = GUID::from_u128(0x2a4b6f25_7a77_4f68_9a15_808b7f4f2cf5);

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(class_id: *const GUID, iid: *const GUID, out: *mut *mut core::ffi::c_void) -> HRESULT {
    unsafe { *out = std::ptr::null_mut() };
    let class_id = unsafe { *class_id };
    if class_id != MODULE_ID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let f: IClassFactory = Factory {}.into();
    let r = unsafe { f.query(iid, out) };
    if r.is_ok() {
        unsafe { *out = f.into_raw() };
    }
    r
}

fn install_folder_from_dll() -> Option<PathBuf> {
    let mut buf = vec![0u16; MAX_PATH as usize];
    unsafe { GetModuleFileNameW(Some(unsafe { DLL_INSTANCE }.into()), &mut buf) };
    while unsafe { GetLastError() } == ERROR_INSUFFICIENT_BUFFER {
        buf = vec![0u16; buf.len() * 2];
        unsafe { GetModuleFileNameW(Some(unsafe { DLL_INSTANCE }.into()), &mut buf) };
    }
    let len = unsafe { u_strlen(buf.as_ptr()) } as usize;
    let p: PathBuf = std::ffi::OsString::from_wide(&buf[..len]).into_string().ok()?.into();
    Some(p.parent()?.parent()?.to_path_buf())
}

#[inline]
fn get_cunning3d_exe_path() -> Option<String> {
    install_folder_from_dll().map(|p| p.join("Cunning3D.exe").to_string_lossy().into_owned())
}

