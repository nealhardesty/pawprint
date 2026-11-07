//! Reading the hostname (SPEC §5.1). `PAWPRINT_HOSTNAME` overrides it.

pub fn detect() -> Result<String, String> {
    if let Ok(name) = std::env::var("PAWPRINT_HOSTNAME")
        && !name.trim().is_empty()
    {
        return Ok(name);
    }
    platform()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| "cannot read the hostname; set PAWPRINT_HOSTNAME".to_string())
}

#[cfg(target_os = "linux")]
fn platform() -> Option<String> {
    // The kernel's nodename, the same value gethostname(2) returns.
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .ok()
}

#[cfg(target_os = "macos")]
fn platform() -> Option<String> {
    // SystemConfiguration's HostName if set, else LocalHostName, the two
    // values `scutil --get` prints. Never gethostname(), which can follow the
    // network (F13).
    use objc2_core_foundation::{CFString, CFType};
    use objc2_system_configuration::{SCDynamicStore, SCPreferences};
    let host_name = || {
        let prefs = SCPreferences::new(None, &CFString::from_static_str("pawprint"), None)?;
        let system = prefs.path_get_value(&CFString::from_static_str("/System/System"))?;
        let key = CFString::from_static_str("HostName");
        // SAFETY: the key is a live CFString; a value, if any, is a CF object
        // owned by `system`, which outlives the borrow.
        let value = unsafe {
            system
                .value((&raw const *key).cast())
                .cast::<CFType>()
                .as_ref()
        }?;
        value.downcast_ref::<CFString>().map(ToString::to_string)
    };
    let local_host_name = || SCDynamicStore::local_host_name(None).map(|s| s.to_string());
    host_name()
        .filter(|s| !s.trim().is_empty())
        .or_else(local_host_name)
}

#[cfg(windows)]
fn platform() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{
        COMPUTER_NAME_FORMAT, ComputerNameDnsHostname, ComputerNameNetBIOS, GetComputerNameExW,
    };
    let query = |kind: COMPUTER_NAME_FORMAT| {
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        // SAFETY: buf is valid for `len` UTF-16 units, and len is updated in place.
        let ok = unsafe { GetComputerNameExW(kind, buf.as_mut_ptr(), &mut len) };
        (ok != 0 && len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    };
    query(ComputerNameDnsHostname).or_else(|| query(ComputerNameNetBIOS))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform() -> Option<String> {
    None
}
