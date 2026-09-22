//! The few registry operations vtype needs on Windows (the Run key for starting with Windows, and
//! finding chrome.exe).

use std::io;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

impl Hive {
    fn key(self) -> HKEY {
        match self {
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn check(code: u32) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

/// Sets a string value (`name` = None for the key's default value), creating the key.
pub fn set_string(hive: Hive, subkey: &str, name: Option<&str>, value: &str) -> io::Result<()> {
    let subkey_w = wide(subkey);
    let name_w = name.map(wide);
    let data = wide(value);
    let mut key: HKEY = null_mut();
    unsafe {
        check(RegCreateKeyExW(
            hive.key(),
            subkey_w.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        ))?;
        let result = RegSetValueExW(
            key,
            name_w.as_ref().map_or(null(), |n| n.as_ptr()),
            0,
            REG_SZ,
            data.as_ptr() as *const u8,
            (data.len() * 2) as u32,
        );
        RegCloseKey(key);
        check(result)
    }
}

/// Reads a string value; `Ok(None)` when the key or value does not exist.
pub fn get_string(hive: Hive, subkey: &str, name: Option<&str>) -> io::Result<Option<String>> {
    let subkey_w = wide(subkey);
    let name_w = name.map(wide);
    let name_ptr = name_w.as_ref().map_or(null(), |n| n.as_ptr());
    let mut size: u32 = 0;
    unsafe {
        let first =
            RegGetValueW(hive.key(), subkey_w.as_ptr(), name_ptr, RRF_RT_REG_SZ, null_mut(), null_mut(), &mut size);
        if first == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(first)?;
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        check(RegGetValueW(
            hive.key(),
            subkey_w.as_ptr(),
            name_ptr,
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        ))?;
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(Some(String::from_utf16_lossy(&buf[..len])))
    }
}

/// Deletes one value. Missing is fine.
pub fn delete_value(hive: Hive, subkey: &str, name: &str) -> io::Result<()> {
    let subkey_w = wide(subkey);
    let name_w = wide(name);
    let code = unsafe { RegDeleteKeyValueW(hive.key(), subkey_w.as_ptr(), name_w.as_ptr()) };
    if code == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        check(code)
    }
}
