//! Dependency regressions for Pylon's Wasmtime feature set.
//! These invalid configurations must fail before guest execution can abort the host.

use wasmtime::{Config, Engine, WasmFeatures};

#[test]
fn rejects_threads_without_compiled_support() {
    let mut config = Config::new();
    config.wasm_features(WasmFeatures::THREADS, true);
    assert!(Engine::new(&config).is_err());
}

#[test]
fn default_engine_remains_valid() {
    assert!(Engine::new(&Config::new()).is_ok());
}
