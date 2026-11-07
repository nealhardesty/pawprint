//! The local wall clock. The core takes the local hour as an input
//! (`Now::local_hour`, SPEC §10.1) because reading the time zone is a
//! platform call.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use pawprint_core::badge::Now;

/// Local date and time, to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl std::fmt::Display for LocalTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

#[cfg(unix)]
pub fn local() -> LocalTime {
    // SAFETY: time(NULL) has no preconditions; localtime_r writes into `tm`,
    // which lives for the whole call.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        LocalTime {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u8,
            day: tm.tm_mday as u8,
            hour: tm.tm_hour as u8,
            minute: tm.tm_min as u8,
            second: tm.tm_sec as u8,
        }
    }
}

#[cfg(windows)]
pub fn local() -> LocalTime {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime fills the SYSTEMTIME it is given.
    let st = unsafe {
        let mut st = std::mem::zeroed();
        GetLocalTime(&mut st);
        st
    };
    LocalTime {
        year: i32::from(st.wYear),
        month: st.wMonth as u8,
        day: st.wDay as u8,
        hour: st.wHour as u8,
        minute: st.wMinute as u8,
        second: st.wSecond as u8,
    }
}

/// What the core needs on every event.
pub fn now() -> Now {
    Now {
        instant: Instant::now(),
        local_hour: local().hour,
    }
}

/// Seed for animation timing (SPEC §7.1: timing need not be reproducible).
pub fn seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0x9e37_79b9, |d| d.as_nanos() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_is_sane() {
        let t = local();
        assert!(t.year >= 2024 && (1..=12).contains(&t.month) && t.hour < 24);
        assert_eq!(t.to_string().len(), "2026-11-06 22:30:01".len());
    }
}
