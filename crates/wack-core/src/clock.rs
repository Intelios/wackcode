//! The user's local time, for the words the Chat area shows: its sidebar's Today and Yesterday,
//! its greeting, and its transcript's time dividers. Through the C library's own time-zone rules,
//! so daylight saving is right without a time-zone database of our own.

/// A moment on the user's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    /// 1 to 31.
    pub day: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

fn tm(ms: u64) -> libc::tm {
    let seconds = (ms / 1000) as libc::time_t;
    // SAFETY: `localtime_r` only writes into the `tm` it is handed, which lives on this stack.
    unsafe {
        let mut time: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut time);
        time
    }
}

/// `ms` (milliseconds since 1970) on the user's clock.
pub fn local(ms: u64) -> Local {
    let time = tm(ms);
    Local {
        year: time.tm_year + 1900,
        month: (time.tm_mon + 1) as u32,
        day: time.tm_mday as u32,
        weekday: time.tm_wday as u32,
        hour: time.tm_hour as u32,
        minute: time.tm_min as u32,
    }
}

/// The local midnight that began `ms`'s day, `days` days earlier (0 for that day itself).
pub fn start_of_day(ms: u64, days: u32) -> u64 {
    let mut time = tm(ms);
    time.tm_hour = 0;
    time.tm_min = 0;
    time.tm_sec = 0;
    time.tm_mday -= days as i32;
    // Let the library decide whether daylight saving was in force that midnight.
    time.tm_isdst = -1;
    // SAFETY: `mktime` reads and normalises the `tm` it is handed, which lives on this stack.
    let seconds = unsafe { libc::mktime(&mut time) };
    (seconds.max(0) as u64) * 1000
}

pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_starts_at_local_midnight_and_steps_back_by_days() {
        let now = 1_790_000_000_000;
        let today = start_of_day(now, 0);
        assert!(today <= now && now - today < 86_400_000 + 3_600_000);
        let midnight = local(today);
        assert_eq!((midnight.hour, midnight.minute), (0, 0));
        assert_eq!(local(today).day, local(now).day);
        let yesterday = start_of_day(now, 1);
        assert!(today - yesterday >= 82_800_000 && today - yesterday <= 90_000_000, "a day, give or take daylight saving");
        assert_eq!(start_of_day(today, 0), today);
    }
}
