//! Exercises the `AsyncSerial`/`AsyncSerialPort` surface over the loopback device.

use alloc::string::ToString;

use ferredge_core::prelude::*;
use ferredge_runtime_embassy::EmbassySerial;

use crate::fakes::LoopbackSerial;

pub async fn run() {
    log::debug!("opening loopback serial port /dev/uart0");
    let serial = EmbassySerial::new();
    serial.register("/dev/uart0", LoopbackSerial::default());
    let config = SerialPortConfig {
        path: "/dev/uart0".to_string(),
        ..SerialPortConfig::default()
    };

    let mut port = serial.open(&config).await.expect("open should succeed");
    assert_eq!(port.write(b"rtu").await, Ok(3));
    let mut buf = [0u8; 8];
    assert_eq!(port.read(&mut buf).await, Ok(3));
    assert_eq!(&buf[..3], b"rtu");
    assert!(serial.open(&config).await.is_err());
}
