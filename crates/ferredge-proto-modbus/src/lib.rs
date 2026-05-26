#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod attributes;
mod codec;
mod convert;
#[cfg(all(test, feature = "integration"))]
mod diagslave_tests;
#[cfg(test)]
mod tests;
mod transport;
mod types;

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
        TokioDatagramSocket as StackDatagramSocket, TokioNet as StackNet,
        TokioRuntime as StackRuntime, TokioSerial as StackSerial,
        TokioSerialPort as StackSerialPort, TokioSocket as StackSocket,
    };
}
#[cfg(feature = "async-std-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_async_std::{
        AsyncStdDatagramSocket as StackDatagramSocket, AsyncStdNet as StackNet,
        AsyncStdRuntime as StackRuntime, AsyncStdSerial as StackSerial,
        AsyncStdSerialPort as StackSerialPort, AsyncStdSocket as StackSocket,
    };
}

pub(crate) use runtime_stack::{
    StackDatagramSocket, StackNet, StackRuntime, StackSerial, StackSerialPort, StackSocket,
};

pub use convert::ModbusBridgeCodec;
pub use types::{
    ModbusCommandConversionError, ModbusCommandPlanner, ModbusDecodedResponse, ModbusDriver,
    ModbusNativePlan, ModbusParserSeed, ModbusRequest, ModbusResponse, ModbusResponseDecoder,
    ModbusResponseDecoderContext,
};
