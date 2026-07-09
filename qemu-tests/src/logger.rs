use cortex_m_semihosting::hprintln;

/// Routes `log` records to the semihosting console, prefixed with their level.
pub(crate) struct SemihostingLogger;

impl log::Log for SemihostingLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        hprintln!("[{:<5}] {}", record.level(), record.args());
    }

    fn flush(&self) {}
}
