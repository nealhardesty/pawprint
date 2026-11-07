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
    // SystemConfiguration's HostName if set, else LocalHostName. Never
    // gethostname(), which can follow the network (F13).
    let get = |key: &str| {
        std::process::Command::new("/usr/sbin/scutil")
            .args(["--get", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .filter(|s| !s.trim().is_empty())
    };
    get("HostName").or_else(|| get("LocalHostName"))
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
