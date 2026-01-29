use std::{fs, path::Path, time::SystemTime};

use anyhow::{Context, Result};
use wasmtime::{
    Engine, Store,
    component::{Component, Linker},
};

use super::{TestHostState, add_wasi_to_linker, add_wasmtime_hosts};

pub fn instantiate_provider(engine: &Engine, wasm_path: &Path) -> Result<()> {
    let canonical = fs::canonicalize(wasm_path).unwrap_or_else(|_| wasm_path.to_path_buf());
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("failed to stat component: {}", canonical.display()))?;
    let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

    eprintln!(
        "[provider-tests] instantiating component:\n  path={}\n  size={} bytes\n  mtime={:?}",
        canonical.display(),
        metadata.len(),
        mtime,
    );

    let component = Component::from_file(engine, &canonical)
        .with_context(|| format!("failed to load component: {}", canonical.display()))?;

    let mut store = Store::new(engine, TestHostState::with_default_secrets());
    let mut linker = Linker::new(engine);
    add_wasi_to_linker(&mut linker);
    add_wasmtime_hosts(&mut linker)
        .context("failed to register greentic:http/client@1.1.0 hosts")?;

    linker
        .instantiate(&mut store, &component)
        .context("component instantiation failed (missing imports?)")?;

    Ok(())
}
