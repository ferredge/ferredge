use std::sync::OnceLock;

use crate::runtime_stack::StackRuntime;

pub fn block_on<F>(future: F) -> F::Output
where
    F: core::future::Future,
{
    static RUNTIME: OnceLock<StackRuntime> = OnceLock::new();
    RUNTIME.get_or_init(StackRuntime::default).block_on(future)
}
