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
#[cfg(feature = "embassy-runtime")]
mod runtime_stack {
    pub use ferredge_runtime_embassy::{
        EmbassyDatagramSocket as StackDatagramSocket, EmbassyDynSerial as StackSerial,
        EmbassyNet as StackNet, EmbassyRuntime as StackRuntime, EmbassySocket as StackSocket,
    };

    /// Serial ports are type-erased on embassy; see `EmbassyDynSerialPort`.
    pub type StackSerialPort =
        ferredge_runtime_embassy::EmbassySerialPort<ferredge_runtime_embassy::EmbassyDynSerialPort>;
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
