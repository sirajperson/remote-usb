use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::error::Result;
use crate::filter::DeviceFilter;
use crate::kmod::{self, ensure_import_modules};
use crate::privilege::require_root;
use crate::usbip_cmd::{self, format_device_table, format_port_table};

/// Load import-side kernel modules.
pub fn prepare() -> Result<()> {
    ensure_import_modules()?;
    for line in kmod::module_status_lines(&["usbip_core", "vhci_hcd"]) {
        println!("{line}");
    }
    println!("Server (import) modules ready.");
    Ok(())
}

/// List devices exported by a remote client host.
pub fn list(host: &str, port: u16) -> Result<()> {
    let devices = usbip_cmd::list_remote(host, port)?;
    println!("Devices on {host}:{port}");
    println!("{}", format_device_table(&devices));
    Ok(())
}

/// Attach a remote device so it appears as a local USB device.
pub fn attach(host: &str, selector: &str, port: u16) -> Result<()> {
    require_root("attach a remote USB device")?;
    ensure_import_modules()?;

    let devices = usbip_cmd::list_remote(host, port)?;
    let dev = usbip_cmd::resolve_selector(&devices, selector)?;
    tracing::info!(
        host,
        busid = %dev.busid,
        vid_pid = %dev.vid_pid,
        "attaching remote device"
    );
    usbip_cmd::attach(host, &dev.busid, port)?;
    println!(
        "Attached {} ({}) from {}:{}.",
        dev.busid, dev.vid_pid, host, port
    );
    if !dev.product.is_empty() {
        println!("Product: {}", dev.product);
    }
    println!(
        "The device should appear in `lsusb` shortly.\n\
         Mass-storage devices typically show up under /dev/disk/by-id/ and may be\n\
         auto-mounted by your desktop/udisks. Check: remote-usb server ports"
    );
    Ok(())
}

/// Detach an imported device by VHCI port number.
pub fn detach(port_num: u32) -> Result<()> {
    require_root("detach an imported USB device")?;
    usbip_cmd::detach(port_num)?;
    println!("Detached port {port_num}.");
    Ok(())
}

/// List currently imported devices.
pub fn ports() -> Result<()> {
    // Prefer having modules loaded, but still try to list.
    if !kmod::is_loaded("vhci_hcd") {
        eprintln!("warning: vhci_hcd is not loaded; run `remote-usb server prepare`");
    }
    let ports = usbip_cmd::port_list()?;
    println!("{}", format_port_table(&ports));
    Ok(())
}

/// Options for continuous direct attachment (`server follow`).
pub struct FollowOptions {
    pub host: String,
    pub port: u16,
    pub filter: DeviceFilter,
    pub interval: Duration,
    /// Detach local ports when the remote device disappears.
    pub detach_missing: bool,
}

/// Continuously attach every matching device exported by the client.
///
/// Pair with `remote-usb client serve --auto` for direct attachment.
pub fn follow(opts: FollowOptions) -> Result<()> {
    require_root("auto-attach remote USB devices")?;
    ensure_import_modules()?;

    println!(
        "Following {}:{port} for direct attachment.\n\
         Filter: {}\n\
         Poll interval: {:.1}s | detach missing: {}\n\
         Press Ctrl+C to stop.",
        opts.host,
        opts.filter.describe(),
        opts.interval.as_secs_f32(),
        opts.detach_missing,
        port = opts.port,
    );

    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
    }) {
        tracing::warn!(error = %e, "could not install Ctrl+C handler");
    }

    // busids we successfully attached (or saw as attached).
    let mut known: HashSet<String> = HashSet::new();

    while running.load(Ordering::SeqCst) {
        match follow_once(&opts, &mut known) {
            Ok(()) => {}
            Err(e) => {
                // Client may be temporarily down; keep retrying.
                tracing::warn!(error = %e, "follow cycle failed; will retry");
                eprintln!("follow: {e} (retrying)");
            }
        }
        sleep_interruptible(opts.interval, &running);
    }

    println!("Stopped following.");
    Ok(())
}

fn follow_once(opts: &FollowOptions, known: &mut HashSet<String>) -> Result<()> {
    let remote = usbip_cmd::list_remote(&opts.host, opts.port)?;
    let ports = usbip_cmd::port_list().unwrap_or_default();
    let attached = usbip_cmd::attached_remote_busids(&ports);

    let remote_busids: HashSet<String> = remote.iter().map(|d| d.busid.clone()).collect();

    // Attach new matching devices.
    for dev in &remote {
        if !opts.filter.allows(dev) {
            continue;
        }
        if attached.contains_key(&dev.busid) {
            known.insert(dev.busid.clone());
            continue;
        }

        match usbip_cmd::attach(&opts.host, &dev.busid, opts.port) {
            Ok(()) => {
                println!(
                    "auto-attach: {} ({}) from {}:{}{}",
                    dev.busid,
                    dev.vid_pid,
                    opts.host,
                    opts.port,
                    if dev.product.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", dev.product)
                    }
                );
                known.insert(dev.busid.clone());
            }
            Err(e) if usbip_cmd::is_already_attached_error(&e) => {
                known.insert(dev.busid.clone());
            }
            Err(e) => {
                tracing::warn!(
                    busid = %dev.busid,
                    error = %e,
                    "auto-attach failed"
                );
                eprintln!("auto-attach failed for {}: {e}", dev.busid);
            }
        }
    }

    // Detach ports whose remote device is gone.
    if opts.detach_missing {
        // Refresh attachment map after possible attaches.
        let ports = usbip_cmd::port_list().unwrap_or_default();
        let attached = usbip_cmd::attached_remote_busids(&ports);

        for (busid, port_num) in &attached {
            if remote_busids.contains(busid) {
                continue;
            }
            // Only detach if we know about it or it matches filter history.
            if !known.contains(busid) {
                // Still detach orphans from this host if URL host matches.
                if !remote_line_matches_host(
                    ports
                        .iter()
                        .find(|p| p.port == *port_num)
                        .map(|p| p.remote.as_str())
                        .unwrap_or(""),
                    &opts.host,
                ) {
                    continue;
                }
            }

            match usbip_cmd::detach(*port_num) {
                Ok(()) => {
                    println!("auto-detach: port {port_num} (remote busid {busid} gone)");
                    known.remove(busid);
                }
                Err(e) => {
                    tracing::warn!(port = port_num, error = %e, "auto-detach failed");
                }
            }
        }

        known.retain(|b| remote_busids.contains(b) || attached.contains_key(b));
    }

    Ok(())
}

fn remote_line_matches_host(remote: &str, host: &str) -> bool {
    // usbip://192.168.1.2:3240/5-2 or similar
    let host_l = host.to_ascii_lowercase();
    remote.to_ascii_lowercase().contains(&host_l)
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
