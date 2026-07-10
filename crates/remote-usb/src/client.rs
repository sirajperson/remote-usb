use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::control::{self, ClientMsg, ControlSession};
use crate::error::Result;
use crate::filter::{self, DeviceFilter};
use crate::kmod::{self, ensure_export_modules};
use crate::privilege::require_root;
use crate::usbip_cmd::{self, format_device_table};

/// Load client modules (physical USB host).
pub fn prepare() -> Result<()> {
    ensure_export_modules()?;
    for line in kmod::module_status_lines(&["usbip_core", "usbip_host"]) {
        println!("{line}");
    }
    println!("Client ready — you can attach local USB devices to a server.");
    Ok(())
}

/// List local USB devices.
pub fn list() -> Result<()> {
    let devices = usbip_cmd::list_local()?;
    println!("{}", format_device_table(&devices));
    Ok(())
}

/// Stop offering a local device.
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

    usbip_cmd::unbind(&busid)?;
    println!("Released {busid}.");
    Ok(())
}

/// Client `attach` options: connect to a waiting server and offer devices.
pub struct AttachOptions {
    pub server_addr: String,
    pub control_port: u16,
    pub usbip_port: u16,
    pub ipv4_only: bool,
    pub ipv6_only: bool,
    pub selector: Option<String>,
    pub auto: bool,
    pub filter: DeviceFilter,
    pub interval: Duration,
    pub unbind_on_exit: bool,
}

/// Attach local USB device(s) to a server that is already running `serve`.
pub fn attach_to_server(opts: AttachOptions) -> Result<()> {
    require_root("attach USB devices to the server")?;
    ensure_export_modules()?;

    // Local USB/IP export listener so the server can pull the device after OFFER.
    let mut child = usbip_cmd::spawn_usbipd(opts.usbip_port, opts.ipv4_only, opts.ipv6_only)?;
    thread::sleep(Duration::from_millis(300));

    let result = if opts.auto {
        attach_auto(&opts)
    } else {
        let sel = opts
            .selector
            .as_deref()
            .expect("selector required without --auto");
        attach_one(&opts, sel)
    };

    let _ = child.kill();
    let _ = child.wait();
    result
}

fn attach_one(opts: &AttachOptions, selector: &str) -> Result<()> {
    let devices = usbip_cmd::list_local()?;
    let dev = usbip_cmd::resolve_selector(&devices, selector)?;

    usbip_cmd::bind(&dev.busid)?;
    println!(
        "Offering {} ({}) to server {}…",
        dev.busid, dev.vid_pid, opts.server_addr
    );

    control::send_to_server(
        &opts.server_addr,
        opts.control_port,
        &ClientMsg::Offer {
            usbip_port: opts.usbip_port,
            busid: dev.busid.clone(),
        },
    )?;

    println!(
        "Server accepted {}.\n\
         Keep this process running while the server uses the device.\n\
         On the server: lsusb\n\
         Press Ctrl+C to detach.",
        dev.busid
    );

    wait_until_ctrl_c();

    println!("Revoking {} from server…", dev.busid);
    let _ = control::send_to_server(
        &opts.server_addr,
        opts.control_port,
        &ClientMsg::Revoke {
            busid: dev.busid.clone(),
        },
    );
    let _ = usbip_cmd::unbind(&dev.busid);
    println!("Done.");
    Ok(())
}

fn attach_auto(opts: &AttachOptions) -> Result<()> {
    println!(
        "Client connecting to server {} (control port {}).\n\
         Auto-attaching matching devices | filter: {} | poll: {:.1}s\n\
         WARNING: without --match this may attach keyboards/mice.\n\
         Press Ctrl+C to stop.",
        opts.server_addr,
        opts.control_port,
        opts.filter.describe(),
        opts.interval.as_secs_f32()
    );

    let mut session = ControlSession::connect(&opts.server_addr, opts.control_port)?;
    let mut offered: HashSet<String> = HashSet::new();

    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let _ = ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
    });

    while running.load(Ordering::SeqCst) {
        match offer_new_devices(opts, &mut session, &mut offered) {
            Ok(n) if n > 0 => tracing::info!(n, "offered new devices to server"),
            Ok(_) => {}
            Err(e) => {
                eprintln!("offer cycle failed: {e} (reconnecting…)");
                thread::sleep(Duration::from_secs(1));
                match ControlSession::connect(&opts.server_addr, opts.control_port) {
                    Ok(s) => session = s,
                    Err(e2) => eprintln!("reconnect failed: {e2}"),
                }
            }
        }
        sleep_interruptible(opts.interval, &running);
    }

    println!("Revoking devices from server…");
    for busid in &offered {
        let _ = session.request(&ClientMsg::Revoke {
            busid: busid.clone(),
        });
        if opts.unbind_on_exit {
            let _ = usbip_cmd::unbind(busid);
        }
    }
    println!("Stopped.");
    Ok(())
}

fn offer_new_devices(
    opts: &AttachOptions,
    session: &mut ControlSession,
    offered: &mut HashSet<String>,
) -> Result<usize> {
    let devices = usbip_cmd::list_local()?;
    let mut n = 0usize;

    offered.retain(|busid| {
        std::path::Path::new(&format!("/sys/bus/usb/devices/{busid}")).exists()
    });

    for dev in devices {
        if !opts.filter.allows(&dev) {
            continue;
        }
        if offered.contains(&dev.busid) {
            continue;
        }
        if !filter::is_exported(&dev.busid) {
            match usbip_cmd::bind(&dev.busid) {
                Ok(()) => {}
                Err(e) if usbip_cmd::is_already_bound_error(&e) => {}
                Err(e) => {
                    tracing::warn!(busid = %dev.busid, error = %e, "bind failed");
                    continue;
                }
            }
        }

        match session.request(&ClientMsg::Offer {
            usbip_port: opts.usbip_port,
            busid: dev.busid.clone(),
        }) {
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
            Err(e) => {
                eprintln!("server rejected {}: {e}", dev.busid);
            }
        }
    }
    Ok(n)
}

fn wait_until_ctrl_c() {
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    let _ = ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
    });
    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));
    }
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
