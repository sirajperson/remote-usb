use crate::error::Result;
use crate::kmod::{self, ensure_import_modules};
use crate::privilege::require_root;
use crate::usbip_cmd::{self, format_port_table};

/// Load server modules (receive side).
pub fn prepare() -> Result<()> {
    ensure_import_modules()?;
    for line in kmod::module_status_lines(&["usbip_core", "vhci_hcd"]) {
        println!("{line}");
    }
    println!("Server modules ready — run `remote-usb serve` to wait for clients.");
    Ok(())
}

/// List devices already received on this server.
pub fn ports() -> Result<()> {
    if !kmod::is_loaded("vhci_hcd") {
        eprintln!("warning: vhci_hcd not loaded; run `sudo remote-usb serve` or `serve prepare`");
    }
    let ports = usbip_cmd::port_list()?;
    println!("{}", format_port_table(&ports));
    Ok(())
}

/// Remove a received device by VHCI port.
pub fn detach(port_num: u32) -> Result<()> {
    require_root("remove a received USB device")?;
    usbip_cmd::detach(port_num)?;
    println!("Removed port {port_num}.");
    Ok(())
}
