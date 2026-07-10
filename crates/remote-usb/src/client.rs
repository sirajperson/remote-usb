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

/// Load client (physical USB) kernel modules.
pub fn prepare() -> Result<()> {
    ensure_export_modules()?;
    for line in kmod::module_status_lines(&["usbip_core", "usbip_host"]) {
        println!("{line}");
    }
    println!("Client ready — this machine can attach USB devices to a server.");
    Ok(())
}

/// List local USB devices on the client.
pub fn list() -> Result<()> {
    let devices = usbip_cmd::list_local()?;
    println!("{}", format_device_table(&devices));
    Ok(())
}

/// Stop offering a local device (return it to local drivers).
pub fn unbind(selector: &str) -> Result<()> {
    require_root("release a USB device")?;
    let devices = usbip_cmd::list_local().unwrap_or_default();
    let busid = if let Ok(dev) = usbip_cmd::resolve_selector(&devices, selector) {
        dev.busid
    } else if !selector.contains(':') {
        selector.to_string()
    } else {
        return Err(crate::error::Error::DeviceNotFound {
            selector: selector.to_string(),
        });
    };

    tracing::info!(%busid, "unbinding device");
    usbip_cmd::unbind(&busid)?;
    println!("Released {busid} back to local use.");
    Ok(())
}

/// Options for client `attach` — attach local USB to a remote server.
pub struct AttachOptions {
    /// Server the client is attaching devices to (for messages / operator clarity).
    pub server_addr: String,
    pub port: u16,
    pub ipv4_only: bool,
    pub ipv6_only: bool,
    /// One device to attach; None when auto.
    pub selector: Option<String>,
    pub auto: bool,
    pub filter: DeviceFilter,
    pub interval: Duration,
    pub unbind_on_exit: bool,
}

/// Client: attach local USB device(s) so the server can receive them.
pub fn attach_to_server(opts: AttachOptions) -> Result<()> {
    require_root("attach USB devices to the server")?;
    ensure_export_modules()?;

    if opts.auto {
        attach_auto(opts)
    } else {
        let selector = opts
            .selector
            .as_deref()
            .expect("selector required without --auto");
        attach_one(&opts, selector)
    }
}

fn attach_one(opts: &AttachOptions, selector: &str) -> Result<()> {
    // Need the export listener up so the server can receive this device.
    let mut child = usbip_cmd::spawn_usbipd(opts.port, opts.ipv4_only, opts.ipv6_only)?;
    thread::sleep(Duration::from_millis(300));

    let devices = usbip_cmd::list_local()?;
    let dev = usbip_cmd::resolve_selector(&devices, selector)?;
    tracing::info!(busid = %dev.busid, vid_pid = %dev.vid_pid, "attaching to server");
    usbip_cmd::bind(&dev.busid)?;

    println!(
        "Client attached {} ({}) to server {}.\n\
         Keep this process running while the server uses the device.\n\
         On the server (if not using --auto):\n\
           sudo remote-usb serve --client <this-client-ip> import {}\n\
         On the server, confirm: lsusb\n\
         Press Ctrl+C to stop offering this device.",
        dev.busid,
        dev.vid_pid,
        opts.server_addr,
        dev.busid
    );
    if !dev.product.is_empty() {
        println!("Product: {}", dev.product);
    }

    // Block until Ctrl+C so usbipd stays up.
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let _ = ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
    });

    while running.load(Ordering::SeqCst) {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(crate::error::Error::Message(format!(
                    "export listener exited: {status}"
                )));
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(e) => {
                return Err(crate::error::Error::Message(format!(
                    "failed to poll export listener: {e}"
                )));
            }
        }
    }

    println!("Stopping…");
    let _ = usbip_cmd::unbind(&dev.busid);
    let _ = child.kill();
    let _ = child.wait();
    println!("Device {} released.", dev.busid);
    Ok(())
}

fn attach_auto(opts: AttachOptions) -> Result<()> {
    println!(
        "Client attaching USB devices to server {} (TCP port {}).\n\
         Filter: {} | poll: {:.1}s\n\
         WARNING: without --match this may attach keyboards/mice too.\n\
         On the server run:\n\
           sudo remote-usb serve 0.0.0.0 --client <this-client-ip> --auto\n\
         Devices then appear in lsusb on the server.\n\
         Press Ctrl+C to stop.",
        opts.server_addr,
        opts.port,
        opts.filter.describe(),
        opts.interval.as_secs_f32()
    );

    let mut child = usbip_cmd::spawn_usbipd(opts.port, opts.ipv4_only, opts.ipv6_only)?;
    thread::sleep(Duration::from_millis(300));

    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
    }) {
        tracing::warn!(error = %e, "could not install Ctrl+C handler");
    }

    let mut offered: HashSet<String> = HashSet::new();

    while running.load(Ordering::SeqCst) {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(crate::error::Error::Message(format!(
                    "export listener exited: {status}"
                )));
            }
            Ok(None) => {}
            Err(e) => {
                return Err(crate::error::Error::Message(format!(
                    "failed to poll export listener: {e}"
                )));
            }
        }

        match offer_matching_once(&opts.filter, &mut offered) {
            Ok(n) if n > 0 => tracing::info!(count = n, "newly attached devices for server"),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "attach scan failed"),
        }

        sleep_interruptible(opts.interval, &running);
    }

    println!("Shutting down…");
    let _ = child.kill();
    let _ = child.wait();

    if opts.unbind_on_exit && !offered.is_empty() {
        println!("Releasing {} device(s)…", offered.len());
        for busid in &offered {
            match usbip_cmd::unbind(busid) {
                Ok(()) => println!("  released {busid}"),
                Err(e) => eprintln!("  failed to release {busid}: {e}"),
            }
        }
    }

    println!("Stopped.");
    Ok(())
}

fn offer_matching_once(filter: &DeviceFilter, offered: &mut HashSet<String>) -> Result<usize> {
    let devices = usbip_cmd::list_local()?;
    let mut n = 0usize;

    offered.retain(|busid| {
        std::path::Path::new(&format!("/sys/bus/usb/devices/{busid}")).exists()
    });

    for dev in devices {
        if !filter.allows(&dev) {
            continue;
        }
        if filter::is_exported(&dev.busid) || offered.contains(&dev.busid) {
            offered.insert(dev.busid.clone());
            continue;
        }

        match usbip_cmd::bind(&dev.busid) {
            Ok(()) => {
                println!(
                    "attached to server: {} ({}){}",
                    dev.busid,
                    dev.vid_pid,
                    if dev.product.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", dev.product)
                    }
                );
                offered.insert(dev.busid);
                n += 1;
            }
            Err(e) if usbip_cmd::is_already_bound_error(&e) => {
                offered.insert(dev.busid);
            }
            Err(e) => {
                tracing::warn!(busid = %dev.busid, error = %e, "attach failed");
            }
        }
    }
    Ok(n)
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
