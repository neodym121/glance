//! API keys in Windows Credential Manager (generic credentials, current user only).
//! Keys are read right before a request and dropped with it.

use windows::Win32::Foundation::{ERROR_NOT_FOUND, FILETIME};
use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
    CredReadW, CredWriteW,
};
use windows::core::{HSTRING, PWSTR};

use crate::error::{Error, Result};
use crate::providers::Secret;

pub fn load(secret: Secret) -> Option<String> {
    let target = HSTRING::from(secret.target());
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(&target, CRED_TYPE_GENERIC, None, &mut credential).ok()?;
        let blob = std::slice::from_raw_parts(
            (*credential).CredentialBlob,
            (*credential).CredentialBlobSize as usize,
        );
        let value = String::from_utf8(blob.to_vec()).ok();
        CredFree(credential as *const _);
        value.filter(|v| !v.is_empty())
    }
}

pub fn store(secret: Secret, value: &str) -> Result<()> {
    let value = value.trim();
    let mut target: Vec<u16> = secret.target().encode_utf16().chain(Some(0)).collect();
    let mut user: Vec<u16> = "api-key".encode_utf16().chain(Some(0)).collect();
    let credential = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(target.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: value.len() as u32,
        CredentialBlob: value.as_ptr() as *mut u8,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user.as_mut_ptr()),
    };
    unsafe { CredWriteW(&credential, 0) }.map_err(|e| {
        Error::new(format!("Could not save the {} key in Credential Manager: {}", secret.name(), Error::from(e)))
    })
}

pub fn delete(secret: Secret) -> Result<()> {
    let target = HSTRING::from(secret.target());
    match unsafe { CredDeleteW(&target, CRED_TYPE_GENERIC, None) } {
        Ok(()) => Ok(()),
        Err(e) if e.code() == ERROR_NOT_FOUND.to_hresult() => Ok(()),
        Err(e) => Err(Error::new(format!(
            "Could not remove the {} key from Credential Manager: {}",
            secret.name(),
            Error::from(e)
        ))),
    }
}
