use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::error::Result;
use crate::filter::{self, DeviceFilter};
use crate::kmod::{self, ensure_export_modules};
use crate::privilege::require_root;
use crate::usbip_cmd::{self, format_device_table};

/// Load export-side kernel modules.
pub fn prepare() -> Result<()> {
    ensure_export_modules()?;
    for line in kmod::module_status_lines(&["usbip_core", "usbip_host"]) {
        println!("{line}");
    }
    println!("Client (export) modules ready.");
    Ok(())
}

/// List local USB devices.
pub fn list() -> Result<()> {
    let devices = usbip_cmd::list_local()?;
    println!("{}", format_device_table(&devices));
    Ok(())
}

/// Bind (export) a device by busid or VID:PID.
pub fn bind(selector: &str) -> Result<()> {
    require_root("bind a USB device for export")?;
    ensure_export_modules()?;

    let devices = usbip_cmd::list_local()?;
    let dev = usbip_cmd::resolve_selector(&devices, selector)?;
    tracing::info!(busid = %dev.busid, vid_pid = %dev.vid_pid, "binding device");
    usbip_cmd::bind(&dev.busid)?;
    println!(
        "Bound {} ({}) for export.{}",
        dev.busid,
        dev.vid_pid,
        if dev.product.is_empty() {
            String::new()
        } else {
            format!(" — {}", dev.product)
        }
    );
    println!("Ensure `remote-usb client serve` is running so the server can attach.");
    Ok(())
}

/// Unbind a previously exported device.
pub fn unbind(selector: &str) -> Result<()> {
    require_root("unbind a USB device")?;
    // Prefer resolving against local list; if the device is bound it may still appear.
    let devices = usbip_cmd::list_local().unwrap_or_default();
    let busid = if let Ok(dev) = usbip_cmd::resolve_selector(&devices, selector) {
        dev.busid
    } else if !selector.contains(':') {
        // Treat as raw busid when list is empty or device vanished.
        selector.to_string()
    } else {
        return Err(crate::error::Error::DeviceNotFound {
            selector: selector.to_string(),
        });
    };

    tracing::info!(%busid, "unbinding device");
    usbip_cmd::unbind(&busid)?;
    println!("Unbound {busid}.");
    Ok(())
}

/// Options for `client serve`.
pub struct ServeOptions {
    pub port: u16,
    pub ipv4_only: bool,
    pub ipv6_only: bool,
    /// Auto-export matching devices as they appear.
    pub auto: bool,
    pub filter: DeviceFilter,
    pub interval: Duration,
    /// On shutdown, unbind devices this process exported.
    pub unbind_on_exit: bool,
}

/// Run usbipd (and optional auto-export loop).
pub fn serve(opts: ServeOptions) -> Result<()> {
    require_root("run the USB/IP export daemon")?;
    ensure_export_modules()?;

    if opts.auto {
        serve_with_auto(opts)
    } else {
        println!(
            "Starting USB/IP export daemon on TCP port {} (plain TCP, no auth).\n\
             Bind devices with: remote-usb client bind <BUSID|VID:PID>\n\
             Or use --auto for direct attachment.\n\
             Press Ctrl+C to stop.",
            opts.port
        );
        usbip_cmd::serve_foreground(opts.port, opts.ipv4_only, opts.ipv6_only)
    }
}

fn serve_with_auto(opts: ServeOptions) -> Result<()> {
    println!(
        "Starting USB/IP export with direct attachment (auto-export).\n\
         Port: {} | Filter: {}\n\
         Poll interval: {:.1}s | Plain TCP, no auth.\n\
         WARNING: Exported devices leave this machine (keyboard/mouse will disconnect).\n\
         Prefer --match VID:PID for specific devices.\n\
         Press Ctrl+C to stop.",
        opts.port,
        opts.filter.describe(),
        opts.interval.as_secs_f32()
    );

    let mut child = usbip_cmd::spawn_usbipd(opts.port, opts.ipv4_only, opts.ipv6_only)?;
    // Brief delay so usbipd is listening before the first bind.
    thread::sleep(Duration::from_millis(300));

    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
    }) {
        tracing::warn!(error = %e, "could not install Ctrl+C handler");
    }

    let mut exported: HashSet<String> = HashSet::new();

    while running.load(Ordering::SeqCst) {
        // Exit if usbipd died.
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(crate::error::Error::Message(format!(
                    "usbipd exited unexpectedly with {status}"
                )));
            }
            Ok(None) => {}
            Err(e) => {
                return Err(crate::error::Error::Message(format!(
                    "failed to poll usbipd: {e}"
                )));
            }
        }

        match auto_export_once(&opts.filter, &mut exported) {
            Ok(n) if n > 0 => tracing::info!(bound = n, "auto-export cycle bound new devices"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "auto-export cycle failed"),
        }

        // Sleep in small steps so Ctrl+C is responsive.
        sleep_interruptible(opts.interval, &running);
    }

    println!("Shutting down...");
    let _ = child.kill();
    let _ = child.wait();

    if opts.unbind_on_exit && !exported.is_empty() {
        println!("Unbinding {} device(s) exported by this process...", exported.len());
        for busid in &exported {
            match usbip_cmd::unbind(busid) {
                Ok(()) => println!("  unbound {busid}"),
                Err(e) => eprintln!("  failed to unbind {busid}: {e}"),
            }
        }
    }

    println!("Stopped.");
    Ok(())
}

/// Bind any local devices that match the filter and are not yet exported.
/// Returns number of newly bound devices.
fn auto_export_once(filter: &DeviceFilter, exported: &mut HashSet<String>) -> Result<usize> {
    let devices = usbip_cmd::list_local()?;
    let mut bound = 0usize;

    // Drop tracking for devices that disappeared.
    exported.retain(|busid| {
        std::path::Path::new(&format!("/sys/bus/usb/devices/{busid}")).exists()
    });

    for dev in devices {
        if !filter.allows(&dev) {
            continue;
        }
        if filter::is_exported(&dev.busid) || exported.contains(&dev.busid) {
            exported.insert(dev.busid.clone());
            continue;
        }

        match usbip_cmd::bind(&dev.busid) {
            Ok(()) => {
                println!(
                    "auto-export: bound {} ({}){}",
                    dev.busid,
                    dev.vid_pid,
                    if dev.product.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", dev.product)
                    }
                );
                exported.insert(dev.busid);
                bound += 1;
            }
            Err(e) if usbip_cmd::is_already_bound_error(&e) => {
                exported.insert(dev.busid);
            }
            Err(e) => {
                tracing::warn!(
                    busid = %dev.busid,
                    error = %e,
                    "auto-export: bind failed"
                );
            }
        }
    }
    Ok(bound)
}

fn sleep_interruptible(total: Duration, running: &AtomicBool) {
    let step = Duration::from_millis(200);
    let mut left = total;
    while running.load(Ordering::SeqCst) && !left.is_zero() {
        let chunk = step.min(left);
        thread::sleep(chunk);
        left = left.saturating_sub(chunk);
    }
}
