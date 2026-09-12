//! Small time helpers shared by the UI.

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;

/// Current local wall-clock time as `HH:MM:SS`.
pub fn clock_now() -> String {
    // GetLocalTime takes no arguments and returns the current local time.
    let st: SYSTEMTIME = unsafe { GetLocalTime() };
    format!("{:02}:{:02}:{:02}", st.wHour, st.wMinute, st.wSecond)
}
