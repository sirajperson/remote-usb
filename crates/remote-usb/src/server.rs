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

/// Load server modules so this machine can receive client USB devices.
pub fn prepare() -> Result<()> {
    ensure_import_modules()?;
    for line in kmod::module_status_lines(&["usbip_core", "vhci_hcd"]) {
        println!("{line}");
    }
    println!("Server ready — this machine can receive USB devices from clients.");
    Ok(())
}

/// List devices a client is currently offering to the server.
pub fn list(client: &str, port: u16) -> Result<()> {
    let devices = usbip_cmd::list_remote(client, port)?;
    println!("Devices offered by client {client}:{port}");
    println!("{}", format_device_table(&devices));
    Ok(())
}

/// Receive one device from a client onto this server.
pub fn attach(client: &str, selector: &str, port: u16) -> Result<()> {
    require_root("receive a USB device from a client")?;
    ensure_import_modules()?;

    let devices = usbip_cmd::list_remote(client, port)?;
    let dev = usbip_cmd::resolve_selector(&devices, selector)?;
    tracing::info!(
        client,
        busid = %dev.busid,
        vid_pid = %dev.vid_pid,
        "receiving device from client"
    );
    usbip_cmd::attach(client, &dev.busid, port)?;
    println!(
        "Server received {} ({}) from client {}.",
        dev.busid, dev.vid_pid, client
    );
    if !dev.product.is_empty() {
        println!("Product: {}", dev.product);
    }
    println!(
        "Device is now local on this server.\n\
         Confirm:  lsusb\n\
         Status:   remote-usb ports\n\
         Storage:  ls -l /dev/disk/by-id/"
    );
    Ok(())
}

/// Remove a received device.
pub fn detach(port_num: u32) -> Result<()> {
    require_root("remove a received USB device")?;
    usbip_cmd::detach(port_num)?;
    println!("Removed port {port_num}.");
    Ok(())
}

/// List devices already received on this server.
pub fn ports() -> Result<()> {
    if !kmod::is_loaded("vhci_hcd") {
        eprintln!("warning: vhci_hcd is not loaded; run `sudo remote-usb serve prepare`");
    }
    let ports = usbip_cmd::port_list()?;
    println!("{}", format_port_table(&ports));
    Ok(())
}

/// Continuously receive devices from one client (`serve --client … --auto`).
pub struct FollowOptions {
    pub host: String,
    pub port: u16,
    pub filter: DeviceFilter,
    pub interval: Duration,
    pub detach_missing: bool,
}

/// Keep receiving every matching device the client attaches.
pub fn follow(opts: FollowOptions) -> Result<()> {
    require_root("receive USB devices from a client")?;
    ensure_import_modules()?;

    println!(
        "Server receiving USB from client {}:{port}.\n\
         Filter: {} | poll: {:.1}s | remove missing: {}\n\
         Received devices appear in lsusb on THIS server.\n\
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

    let mut known: HashSet<String> = HashSet::new();

    while running.load(Ordering::SeqCst) {
        match follow_once(&opts, &mut known) {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(error = %e, "receive cycle failed; will retry");
                eprintln!("serve: {e} (retrying)");
            }
        }
        sleep_interruptible(opts.interval, &running);
    }

    println!("Server stopped receiving.");
    Ok(())
}

fn follow_once(opts: &FollowOptions, known: &mut HashSet<String>) -> Result<()> {
    let remote = usbip_cmd::list_remote(&opts.host, opts.port)?;
    let ports = usbip_cmd::port_list().unwrap_or_default();
    let attached = usbip_cmd::attached_remote_busids(&ports);

    let remote_busids: HashSet<String> = remote.iter().map(|d| d.busid.clone()).collect();

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
                    "received from client: {} ({}){}",
                    dev.busid,
                    dev.vid_pid,
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
                tracing::warn!(busid = %dev.busid, error = %e, "receive failed");
                eprintln!("receive failed for {}: {e}", dev.busid);
            }
        }
    }

    if opts.detach_missing {
        let ports = usbip_cmd::port_list().unwrap_or_default();
        let attached = usbip_cmd::attached_remote_busids(&ports);

        for (busid, port_num) in &attached {
            if remote_busids.contains(busid) {
                continue;
            }
            if !known.contains(busid) {
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
                    println!("removed port {port_num} (client device {busid} gone)");
                    known.remove(busid);
                }
                Err(e) => {
                    tracing::warn!(port = port_num, error = %e, "remove failed");
                }
            }
        }

        known.retain(|b| remote_busids.contains(b) || attached.contains_key(b));
    }

    Ok(())
}

fn remote_line_matches_host(remote: &str, host: &str) -> bool {
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
