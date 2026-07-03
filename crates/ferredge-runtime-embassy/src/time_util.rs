pub(crate) fn to_embassy_duration(duration: core::time::Duration) -> embassy_time::Duration {
    embassy_time::Duration::from_micros(u64::try_from(duration.as_micros()).unwrap_or(u64::MAX))
}
