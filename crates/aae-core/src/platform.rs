//! What differs between the systems AAE runs on, where the standard library
//! doesn't cover it.

use std::path::Path;

/// A moment as this computer's clock shows it.
pub(crate) struct LocalTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// A time in seconds since 1970 as this computer's local time.
#[cfg(unix)]
pub(crate) fn local_time(secs: u64) -> Option<LocalTime> {
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
    })
}

/// A time in seconds since 1970 as this computer's local time.
#[cfg(windows)]
pub(crate) fn local_time(secs: u64) -> Option<LocalTime> {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // Windows counts in tenths of a microsecond since 1601.
    let ticks = (secs + 11_644_473_600) * 10_000_000;
    let file = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut local: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        if FileTimeToSystemTime(&file, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0
        {
            return None;
        }
    }
    Some(LocalTime {
        year: local.wYear as i32,
        month: local.wMonth as u32,
        day: local.wDay as u32,
        hour: local.wHour as u32,
        minute: local.wMinute as u32,
        second: local.wSecond as u32,
    })
}

/// Free space on the disk holding `path`, in bytes.
#[cfg(unix)]
pub(crate) fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// Free space on the disk holding `path`, in bytes.
#[cfg(windows)]
pub(crate) fn free_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

/// Starting a program without a console window. On Windows, every program a
/// windowed app starts opens one unless told not to, which would flash up each
/// time AAE runs adb.
pub(crate) trait NoConsole {
    fn no_console(&mut self) -> &mut Self;
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

impl NoConsole for std::process::Command {
    fn no_console(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

impl NoConsole for tokio::process::Command {
    fn no_console(&mut self) -> &mut Self {
        #[cfg(windows)]
        self.creation_flags(CREATE_NO_WINDOW);
        self
    }
}

/// The computer's memory, in bytes, where AAE has no other way to ask.
#[cfg(windows)]
pub(crate) fn memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys)
}

#[cfg(target_os = "macos")]
pub(crate) fn memory() -> Option<u64> {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let found = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    found.then_some(value)
}

#[cfg(target_os = "linux")]
pub(crate) fn memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = info
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub(crate) fn memory() -> Option<u64> {
    None
}

/// True on Windows on ARM, where Google's Android emulator doesn't run: it's
/// only made for Intel and AMD processors on Windows, and running it through
/// Windows' x64 emulation leaves it no hypervisor.
#[cfg(windows)]
pub(crate) fn windows_on_arm() -> bool {
    use windows_sys::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_ARM64;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    let (mut process, mut native) = (0u16, 0u16);
    let asked = unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) };
    asked != 0 && native == IMAGE_FILE_MACHINE_ARM64
}

/// The computer's graphics adapters, by name.
#[cfg(windows)]
pub(crate) fn graphics_adapters() -> Vec<String> {
    use windows_sys::Win32::Graphics::Gdi::{DISPLAY_DEVICEW, EnumDisplayDevicesW};
    let mut names: Vec<String> = Vec::new();
    for index in 0.. {
        let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) } == 0 {
            break;
        }
        let end = device
            .DeviceString
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(128);
        let name = String::from_utf16_lossy(&device.DeviceString[..end])
            .trim()
            .to_string();
        // One entry per screen: each adapter once.
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}
