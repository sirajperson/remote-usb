use std::fs;
use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};
use crate::privilege::require_root;

/// Check whether a kernel module is loaded via `/sys/module/<name>`.
pub fn is_loaded(module: &str) -> bool {
    Path::new("/sys/module").join(module.replace('-', "_")).exists()
}

/// Ensure a kernel module is loaded, invoking `modprobe` if needed.
pub fn ensure_loaded(module: &str) -> Result<()> {
    let sys_name = module.replace('-', "_");
    if is_loaded(&sys_name) {
        tracing::debug!(module = %sys_name, "kernel module already loaded");
        return Ok(());
    }

    require_root(&format!("load kernel module '{sys_name}'"))?;

    tracing::info!(module = %sys_name, "loading kernel module");
    let output = Command::new("modprobe")
        .arg(&sys_name)
        .output()
        .map_err(|e| Error::ModuleLoad {
            module: sys_name.clone(),
            detail: format!("failed to run modprobe: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::ModuleLoad {
            module: sys_name,
            detail: if stderr.is_empty() {
                format!("modprobe exited with {}", output.status)
            } else {
                stderr
            },
        });
    }

    if !is_loaded(&sys_name) {
        return Err(Error::ModuleLoad {
            module: sys_name,
            detail: "modprobe succeeded but /sys/module entry is missing".into(),
        });
    }

    Ok(())
}

/// Client (export) side modules: core + host.
pub fn ensure_export_modules() -> Result<()> {
    ensure_loaded("usbip_core")?;
    ensure_loaded("usbip_host")?;
    Ok(())
}

/// Server (import) side modules: core + vhci.
pub fn ensure_import_modules() -> Result<()> {
    ensure_loaded("usbip_core")?;
    ensure_loaded("vhci_hcd")?;
    Ok(())
}

/// Human-readable status of modules relevant to a role.
pub fn module_status_lines(modules: &[&str]) -> Vec<String> {
    modules
        .iter()
        .map(|m| {
            let name = m.replace('-', "_");
            let state = if is_loaded(&name) { "loaded" } else { "not loaded" };
            format!("{name}: {state}")
        })
        .collect()
}

/// Best-effort read of module version/srcversion if present.
#[allow(dead_code)]
pub fn module_srcversion(module: &str) -> Option<String> {
    let path = Path::new("/sys/module")
        .join(module.replace('-', "_"))
        .join("srcversion");
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_loaded_does_not_panic_on_missing() {
        assert!(!is_loaded("definitely_not_a_real_module_xyzzy"));
    }
}
