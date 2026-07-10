//! remote-usb — share USB devices over the network on Linux (USB/IP).
//!
//! CLI model:
//! - **Default** = machine with the physical USB (export).
//! - **`server [addr]`** = run the export daemon (listens for peers).
//! - **`host <addr> …`** = use devices from a remote export host (import).

mod client;
mod error;
mod filter;
mod kmod;
mod privilege;
mod server;
mod usbip_cmd;

use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::error::{Error, Result};
use crate::filter::DeviceFilter;

const DEFAULT_PORT: u16 = 3240;
const DEFAULT_INTERVAL_SECS: f32 = 2.0;
const DEFAULT_BIND: &str = "0.0.0.0";

const TOP_ABOUT: &str = "Share local USB devices over the network, or use USB devices from another host";

const TOP_LONG_ABOUT: &str = "\
Share USB devices over a trusted network using Linux USB/IP.

MENTAL MODEL
  This machine has the USB stick plugged in  →  you are the export side (default).
  You want to use a USB stick plugged in elsewhere →  talk to that host.

  remote-usb list / bind / server     export side (default — no extra noun)
  remote-usb host <IP> …              import side (use devices from that host)

SECURITY
  Plain TCP (default port 3240), no encryption or authentication.
  Trusted LAN/VPN only. Do not expose the port to the internet.

QUICK START — DIRECT ATTACHMENT
  On the machine with the USB device:

    sudo remote-usb server --auto --match 14cd:1212

  On the machine that should use it:

    sudo remote-usb host 192.168.1.10 --auto --match 14cd:1212

QUICK START — MANUAL
  Export side:

    remote-usb list
    sudo remote-usb server 0.0.0.0
    sudo remote-usb bind 1-6

  Import side:

    remote-usb host 192.168.1.10 list
    sudo remote-usb host 192.168.1.10 bind 1-6
    remote-usb ports
";

const TOP_AFTER_HELP: &str = "\
Examples:
  # Export side (USB plugged in here)
  remote-usb list
  sudo remote-usb server --auto --match 0781:5581
  sudo remote-usb server 0.0.0.0
  sudo remote-usb bind 1-6
  sudo remote-usb unbind 1-6

  # Import side (use remote USB)
  remote-usb host 192.168.1.10 list
  sudo remote-usb host 192.168.1.10 bind 1-6
  sudo remote-usb host 192.168.1.10 --auto --match 0781:5581
  remote-usb ports
  sudo remote-usb detach 0

  remote-usb --help
  remote-usb server --help
  remote-usb host --help

Environment:
  REMOTE_USB_PORT   Default TCP port (3240)
  RUST_LOG          Log filter
";

#[derive(Debug, Parser)]
#[command(
    name = "remote-usb",
    version,
    about = TOP_ABOUT,
    long_about = TOP_LONG_ABOUT,
    after_help = TOP_AFTER_HELP,
    after_long_help = TOP_AFTER_HELP,
    arg_required_else_help = true,
    propagate_version = true
)]
struct Cli {
    /// More logging (-v info, -vv debug). Also honors RUST_LOG.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// List local USB devices (export side)
    #[command(
        long_about = "List USB devices plugged into this machine.\n\n\
Use the BUSID (e.g. 1-6) or VID:PID (e.g. 14cd:1212) with bind/unbind.",
        after_help = "Examples:\n  remote-usb list"
    )]
    List,

    /// Share a local USB device (export it so a remote host can use it)
    #[command(
        long_about = "Export a local USB device via USB/IP.\n\n\
Requires root and a running `remote-usb server`.\n\
SELECTOR: busid (1-6) or VID:PID (14cd:1212).",
        after_help = "Examples:\n  \
sudo remote-usb bind 1-6\n  \
sudo remote-usb bind 14cd:1212"
    )]
    Bind {
        /// Busid (1-6) or VID:PID (14cd:1212)
        selector: String,
    },

    /// Stop sharing a local USB device
    #[command(
        long_about = "Stop exporting a device so local drivers can use it again.",
        after_help = "Examples:\n  sudo remote-usb unbind 1-6"
    )]
    Unbind {
        /// Busid or VID:PID
        selector: String,
    },

    /// Load kernel modules for exporting USB (usbip_host)
    #[command(
        long_about = "Load usbip_core + usbip_host on this machine (export side)."
    )]
    Prepare,

    /// List USB devices already imported on this machine
    #[command(
        long_about = "Show remote devices currently attached via vhci (import side).\n\
Port numbers are used with `detach`.",
        after_help = "Examples:\n  remote-usb ports"
    )]
    Ports,

    /// Detach an imported USB device (import side)
    #[command(
        long_about = "Detach a device previously attached from a remote host.\n\
PORT is the VHCI port from `remote-usb ports` (0, 1, …), not the TCP port.",
        after_help = "Examples:\n  remote-usb ports\n  sudo remote-usb detach 0"
    )]
    Detach {
        /// VHCI port from `remote-usb ports`
        port: u32,
    },

    /// Run the export daemon — listen so other machines can use your USB devices
    #[command(
        about = "Listen and share USB devices (export daemon)",
        long_about = "\
Start the USB/IP export daemon on this machine (the one with physical USB).

BIND_ADDR defaults to 0.0.0.0 (all interfaces). usbipd listens on all addresses;
use a firewall to restrict access. TCP port defaults to 3240.

Without --auto, share devices with `remote-usb bind`.
With --auto, matching devices are shared as they appear.

On the other machine, use:
  remote-usb host <this-machine-ip> …

WARNING: --auto without --match shares ALL non-hub USB devices (keyboard/mouse too).",
        after_help = "\
Examples:
  # Listen on all interfaces; bind devices separately
  sudo remote-usb server
  sudo remote-usb server 0.0.0.0
  sudo remote-usb bind 14cd:1212

  # Auto-share a flash drive
  sudo remote-usb server --auto --match 0781:5581

  # Custom port
  sudo remote-usb server 0.0.0.0 --port 3241 --auto --match 14cd:1212
"
    )]
    Server {
        /// Address to advertise / listen intent (default 0.0.0.0 = all interfaces)
        #[arg(default_value = DEFAULT_BIND)]
        bind_addr: String,

        /// TCP port (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,

        /// IPv4 only
        #[arg(long)]
        ipv4: bool,

        /// IPv6 only
        #[arg(long)]
        ipv6: bool,

        /// Auto-share matching local USB devices as they appear
        #[arg(long)]
        auto: bool,

        /// Only auto-share these VID:PID values (repeatable)
        #[arg(long = "match", value_name = "VID:PID")]
        match_ids: Vec<String>,

        /// Never auto-share these VID:PID values (repeatable)
        #[arg(long = "exclude", value_name = "VID:PID")]
        exclude_ids: Vec<String>,

        /// Also auto-share USB hubs
        #[arg(long)]
        include_hubs: bool,

        /// Seconds between scans when --auto is set
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: f32,

        /// Keep devices shared after exit (default: unbind on exit with --auto)
        #[arg(long)]
        no_unbind_on_exit: bool,
    },

    /// Use USB devices from a remote export host (import side)
    #[command(
        about = "Use USB devices from another machine",
        long_about = "\
Connect to a machine running `remote-usb server` and use its USB devices.

  remote-usb host <IP> list              devices available on that host
  remote-usb host <IP> bind <device>     attach one device here
  remote-usb host <IP> --auto            keep attaching as devices appear

Also load import modules once:
  sudo remote-usb host <IP> prepare
",
        after_help = "\
Examples:
  remote-usb host 192.168.1.10 list
  sudo remote-usb host 192.168.1.10 bind 1-6
  sudo remote-usb host 192.168.1.10 bind 14cd:1212
  sudo remote-usb host 192.168.1.10 --auto --match 14cd:1212
  sudo remote-usb host 192.168.1.10 prepare
",
        arg_required_else_help = true
    )]
    Host {
        /// IP or hostname of the machine running `remote-usb server`
        addr: String,

        /// TCP port on the remote host
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,

        /// Continuously attach devices exported by the remote host
        #[arg(long)]
        auto: bool,

        /// Only auto-attach these VID:PID values (repeatable)
        #[arg(long = "match", value_name = "VID:PID")]
        match_ids: Vec<String>,

        /// Never auto-attach these VID:PID values
        #[arg(long = "exclude", value_name = "VID:PID")]
        exclude_ids: Vec<String>,

        /// Also attach USB hubs
        #[arg(long)]
        include_hubs: bool,

        /// Seconds between polls when --auto is set
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: f32,

        /// Do not detach when a remote device disappears
        #[arg(long)]
        no_detach_missing: bool,

        #[command(subcommand)]
        command: Option<HostCmd>,
    },
}

#[derive(Debug, Subcommand)]
enum HostCmd {
    /// Load kernel modules for importing USB (vhci_hcd)
    #[command(long_about = "Load usbip_core + vhci_hcd on this machine (import side).")]
    Prepare,

    /// List devices currently shared by the remote host
    #[command(
        long_about = "List USB devices exported by the remote `remote-usb server`.",
        after_help = "Examples:\n  remote-usb host 192.168.1.10 list"
    )]
    List,

    /// Attach a remote device so it appears as local USB
    #[command(
        name = "bind",
        visible_alias = "attach",
        long_about = "Attach one device from the remote host to this machine.\n\n\
After this, the device shows up in lsusb / as a block device for storage.\n\
SELECTOR: busid or VID:PID from `host <ip> list`.",
        after_help = "Examples:\n  \
sudo remote-usb host 192.168.1.10 bind 1-6\n  \
sudo remote-usb host 192.168.1.10 bind 14cd:1212\n  \
sudo remote-usb host 192.168.1.10 attach 14cd:1212"
    )]
    Bind {
        /// Busid or VID:PID on the remote host
        selector: String,
    },
}

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    if let Err(e) = run(cli.command) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run(command: Commands) -> Result<()> {
    match command {
        Commands::List => client::list(),
        Commands::Bind { selector } => client::bind(&selector),
        Commands::Unbind { selector } => client::unbind(&selector),
        Commands::Prepare => client::prepare(),
        Commands::Ports => server::ports(),
        Commands::Detach { port } => server::detach(port),

        Commands::Server {
            bind_addr,
            port,
            ipv4,
            ipv6,
            auto,
            match_ids,
            exclude_ids,
            include_hubs,
            interval,
            no_unbind_on_exit,
        } => {
            if ipv4 && ipv6 {
                return Err(Error::Message(
                    "cannot specify both --ipv4 and --ipv6".into(),
                ));
            }
            if interval <= 0.0 {
                return Err(Error::Message("--interval must be positive".into()));
            }
            validate_bind_addr(&bind_addr)?;
            let filter = DeviceFilter::from_cli(&match_ids, &exclude_ids, include_hubs)?;
            client::serve(client::ServeOptions {
                bind_addr,
                port,
                ipv4_only: ipv4,
                ipv6_only: ipv6,
                auto,
                filter,
                interval: Duration::from_secs_f32(interval),
                unbind_on_exit: !no_unbind_on_exit,
            })
        }

        Commands::Host {
            addr,
            port,
            auto,
            match_ids,
            exclude_ids,
            include_hubs,
            interval,
            no_detach_missing,
            command,
        } => {
            if auto && command.is_some() {
                return Err(Error::Message(
                    "use either --auto or a subcommand (list/bind/prepare), not both".into(),
                ));
            }
            if auto {
                if interval <= 0.0 {
                    return Err(Error::Message("--interval must be positive".into()));
                }
                let filter = DeviceFilter::from_cli(&match_ids, &exclude_ids, include_hubs)?;
                return server::follow(server::FollowOptions {
                    host: addr,
                    port,
                    filter,
                    interval: Duration::from_secs_f32(interval),
                    detach_missing: !no_detach_missing,
                });
            }
            match command {
                Some(HostCmd::Prepare) => server::prepare(),
                Some(HostCmd::List) => server::list(&addr, port),
                Some(HostCmd::Bind { selector }) => server::attach(&addr, &selector, port),
                None => Err(Error::Message(
                    "specify a subcommand (list, bind, prepare) or --auto\n\
                     try: remote-usb host --help"
                        .into(),
                )),
            }
        }
    }
}

fn validate_bind_addr(addr: &str) -> Result<()> {
    if addr.is_empty() {
        return Err(Error::Message("bind address must not be empty".into()));
    }
    // usbipd has no bind-address flag; we accept the addr for CLI clarity.
    // Specific IPs are a firewall concern.
    if addr != "0.0.0.0" && addr != "::" && addr != "*" {
        tracing::info!(
            %addr,
            "note: usbipd listens on all interfaces; restrict access with a firewall if needed"
        );
    }
    Ok(())
}

fn init_tracing(verbose: u8) {
    let default_level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
