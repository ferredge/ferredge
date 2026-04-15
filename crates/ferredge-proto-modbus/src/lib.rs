#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod attributes;
mod codec;
mod convert;
mod transport;
mod types;
#[cfg(all(test, feature = "diagslave-tests"))]
mod diagslave_tests;
#[cfg(test)]
mod tests;

#[cfg(all(feature = "tokio-runtime", feature = "async-std-runtime"))]
compile_error!("ferredge-proto-modbus supports only one std runtime stack feature at a time");
#[cfg(not(any(
    feature = "tokio-runtime",
    feature = "async-std-runtime",
    feature = "embassy-runtime"
)))]
compile_error!("ferredge-proto-modbus requires one runtime stack feature");
#[cfg(feature = "embassy-runtime")]
compile_error!("ferredge-proto-modbus does not support embassy-runtime yet");

#[cfg(feature = "tokio-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_tokio::{
        TokioDatagramSocket as StackDatagramSocket, TokioNet as StackNet, TokioSerial as StackSerial,
        TokioSerialPort as StackSerialPort, TokioSocket as StackSocket,
    };
}
#[cfg(feature = "async-std-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_async_std::{
        AsyncStdDatagramSocket as StackDatagramSocket, AsyncStdNet as StackNet, AsyncStdSerial as StackSerial,
        AsyncStdSerialPort as StackSerialPort, AsyncStdSocket as StackSocket,
    };
}

pub(crate) use runtime_stack::{
    StackDatagramSocket, StackNet, StackSerial, StackSerialPort, StackSocket,
};

pub use types::{
    ModbusCommandConversionError, ModbusCommandRef, ModbusDriver, ModbusParserSeed, ModbusRequest,
    ModbusResponse, ModbusResponseDecoder,
};
