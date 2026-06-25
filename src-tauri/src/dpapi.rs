//! Windows DPAPI (CryptProtectData) — аналог Electron safeStorage для базового слоя секретов.

#[cfg(windows)]
pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            0,
            &mut output,
        )
        .map_err(|e| e.to_string())?;
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
        Ok(out)
    }
}

#[cfg(windows)]
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let mut descr = PWSTR::null();
        CryptUnprotectData(
            &input,
            Some(&mut descr),
            None,
            None,
            None,
            0,
            &mut output,
        )
        .map_err(|e| e.to_string())?;
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
        if !descr.is_null() {
            let _ = LocalFree(HLOCAL(descr.0 as *mut _));
        }
        Ok(out)
    }
}

#[cfg(not(windows))]
pub fn protect(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI доступен только на Windows".into())
}

#[cfg(not(windows))]
pub fn unprotect(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI доступен только на Windows".into())
}
