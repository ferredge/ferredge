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

#[cfg(any(
    all(feature = "tokio-runtime", feature = "async-std-runtime"),
    all(feature = "tokio-runtime", feature = "embassy-runtime"),
    all(feature = "async-std-runtime", feature = "embassy-runtime")
))]
compile_error!("ferredge-proto-modbus supports only one runtime stack feature at a time");
#[cfg(not(any(
    feature = "tokio-runtime",
    feature = "async-std-runtime",
    feature = "embassy-runtime"
)))]
compile_error!("ferredge-proto-modbus requires one runtime stack feature");
#[cfg(feature = "tokio-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_tokio::{
        TokioNet as StackNet, TokioRuntime as StackRuntime, TokioSerial as StackSerial,
    };
}
#[cfg(feature = "async-std-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_async_std::{
        AsyncStdNet as StackNet, AsyncStdRuntime as StackRuntime, AsyncStdSerial as StackSerial,
    };
}
#[cfg(feature = "embassy-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_embassy::{
        EmbassyDynSerial as StackSerial, EmbassyNet as StackNet, EmbassyRuntime as StackRuntime,
    };
}

pub(crate) use runtime_stack::{StackNet, StackRuntime, StackSerial};

pub use convert::ModbusBridgeCodec;
pub use transport::{
    ModbusTransport, SerialTransport, StackSession, StackTransport, TcpTransport, UdpTransport,
};
pub use types::{
    ModbusCommandConversionError, ModbusCommandPlanner, ModbusDecodedResponse, ModbusDriver,
    ModbusNativePlan, ModbusParserSeed, ModbusRequest, ModbusResponse, ModbusResponseDecoder,
    ModbusResponseDecoderContext,
};
