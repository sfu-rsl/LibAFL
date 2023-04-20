//! This is a basic SymCC runtime.
//! It traces the execution to the shared memory region that should be passed through the environment by the fuzzer process.
//! Additionally, it concretizes all floating point operations for simplicity.
//! Refer to the `symcc_runtime` crate documentation for building your own runtime.

// The lib needs to be named SymRuntime for SymCC to find it
#![allow(non_snake_case)]

use libafl::prelude::{AsMutSlice, AsSlice, ShMem, ShMemProvider, StdShMemProvider};
use symcc_runtime::{
    export_runtime,
    filter::{CallStackCoverage, Initialization, NoFloat},
    tracing::{self, StdShMemMessageFileWriter, TracingRuntime},
    NopRuntime, OptionalRuntime, Runtime,
};

const EDGES_MAP_ENV: &str = "EDGES_MAP_SHMEM";
const NO_SYMBOlIC_ENV: &str = "SYMCC_NO_SYMBOLIC_INPUT";

export_runtime!(
    NoFloat => NoFloat;
    CallStackCoverage::default() => CallStackCoverage; // QSym-style expression pruning
    Initialization::new(initialize) => Initialization;
    if std::env::var(NO_SYMBOlIC_ENV).map(|v| v == "1").unwrap_or(false) {
        OptionalRuntime::new(None)
    } else {
        OptionalRuntime::new(Some(TracingRuntime::new(
            StdShMemMessageFileWriter::from_stdshmem_default_env()
                .expect("unable to construct tracing runtime writer. (missing env?)"),
            false,
        )))
    } => OptionalRuntime<TracingRuntime>
);

fn initialize() {
}

#[no_mangle]
pub unsafe extern "C" fn __xsanitizer_cov_trace_pc_guard_init(mut start: *mut u32, stop: *mut u32) {
    if std::env::var(EDGES_MAP_ENV).is_ok() {
        unsafe {
            let shmem = &mut StdShMemProvider::new()
                .expect("Can't access any shared memory.")
                .existing_from_env(EDGES_MAP_ENV)
                .expect("Couldn't connect to edges shared memory.");
            libafl_targets::EDGES_MAP_PTR = shmem.as_mut_slice().as_mut_ptr();
            libafl_targets::EDGES_MAP_PTR_NUM = shmem.len();
        }
    } else {
        println!("No edges map shared memory found.");
    }

    libafl_targets::sancov_pcguard::__sanitizer_cov_trace_pc_guard_init(start, stop)
}

#[no_mangle]
pub unsafe extern "C" fn __xsanitizer_cov_trace_pc_guard(guard: *mut u32) {
    libafl_targets::sancov_pcguard::__sanitizer_cov_trace_pc_guard(guard)
}
