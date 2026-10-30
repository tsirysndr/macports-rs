//! Reentrant native account lookup; no shared libc lookup buffers or subprocesses.
use std::{
    ffi::{CStr, CString},
    io,
    mem::MaybeUninit,
    ptr,
};

pub(crate) struct Account {
    pub id: u32,
    pub name: String,
    pub home: Option<std::path::PathBuf>,
}
pub(crate) fn by_id(id: u32, group: bool) -> io::Result<Option<Account>> {
    lookup(Some(id), None, group)
}
pub(crate) fn by_name(name: &str, group: bool) -> io::Result<Option<Account>> {
    let name = CString::new(name).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
    lookup(None, Some(&name), group)
}
fn lookup(id: Option<u32>, name: Option<&CStr>, group: bool) -> io::Result<Option<Account>> {
    let mut buffer = vec![0u8; 1024];
    loop {
        let (status, record) = unsafe {
            // All output pointers and the scratch buffer remain live until the
            // returned name has been copied. Read records only on success with
            // a non-null result; libc owns no shared storage in these APIs.
            if group {
                let mut storage = MaybeUninit::<libc::group>::uninit();
                let mut result = ptr::null_mut();
                let status = if let Some(id) = id {
                    libc::getgrgid_r(
                        id,
                        storage.as_mut_ptr(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len(),
                        &mut result,
                    )
                } else {
                    libc::getgrnam_r(
                        name.unwrap().as_ptr(),
                        storage.as_mut_ptr(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len(),
                        &mut result,
                    )
                };
                let record = if status == 0 && !result.is_null() {
                    Some((
                        (*result).gr_gid,
                        CStr::from_ptr((*result).gr_name).to_bytes().to_vec(),
                        None,
                    ))
                } else {
                    None
                };
                (status, record)
            } else {
                let mut storage = MaybeUninit::<libc::passwd>::uninit();
                let mut result = ptr::null_mut();
                let status = if let Some(id) = id {
                    libc::getpwuid_r(
                        id,
                        storage.as_mut_ptr(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len(),
                        &mut result,
                    )
                } else {
                    libc::getpwnam_r(
                        name.unwrap().as_ptr(),
                        storage.as_mut_ptr(),
                        buffer.as_mut_ptr().cast(),
                        buffer.len(),
                        &mut result,
                    )
                };
                let record = if status == 0 && !result.is_null() {
                    Some((
                        (*result).pw_uid,
                        CStr::from_ptr((*result).pw_name).to_bytes().to_vec(),
                        Some(CStr::from_ptr((*result).pw_dir).to_bytes().to_vec()),
                    ))
                } else {
                    None
                };
                (status, record)
            }
        };
        if status == libc::ERANGE && buffer.len() < 16 * 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        return record
            .map(|(id, name, home)| {
                use std::os::unix::ffi::OsStringExt;
                String::from_utf8(name)
                    .map(|name| Account {
                        id,
                        name,
                        home: home.map(|bytes| std::ffi::OsString::from_vec(bytes).into()),
                    })
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
            })
            .transpose();
    }
}
