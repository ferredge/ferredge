#![cfg_attr(not(test), allow(dead_code))]

#[cfg(all(feature = "tokio-runtime", feature = "async-std-runtime"))]
compile_error!("ferredge-test-support supports only one runtime stack feature at a time");
#[cfg(not(any(feature = "tokio-runtime", feature = "async-std-runtime")))]
compile_error!("ferredge-test-support requires one runtime stack feature");

pub mod mosquitto;
pub mod net;
pub mod process;
pub mod runtime;
pub mod serial;
pub mod wait;

#[cfg(feature = "tokio-runtime")]
pub(crate) mod runtime_stack {
    pub use ferredge_runtime_tokio::TokioRuntime as StackRuntime;
}

#[cfg(feature = "async-std-runtime")]
pub(crate) mod runtime_stack {
    pub use ferredge_runtime_async_std::AsyncStdRuntime as StackRuntime;
}

pub mod diagslave;
