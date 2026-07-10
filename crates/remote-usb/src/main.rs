//! remote-usb: share USB devices from a client host to a server host via Linux USB/IP.
//!
//! Terminology:
//! - **client** — machine with the physical USB device (export / USB/IP “server”)
//! - **server** — machine that attaches and uses the remote device (import / USB/IP “client”)

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

const TOP_ABOUT: &str = "Share USB devices from a client host to a server host over Linux USB/IP";

const TOP_LONG_ABOUT: &str = "\
Share USB devices from a client host to a server host over Linux USB/IP.

OVERVIEW
  remote-usb is a thin wrapper around the kernel USB/IP stack (usbip / usbipd).
  It does not reimplement the wire protocol; it loads modules, runs the daemon,
  and automates bind/attach with clearer selectors and optional auto mode.

ROLES (our names — opposite of classic USB/IP docs)
  client   Machine where the USB device is physically plugged in (export side).
           Uses kernel modules: usbip_core, usbip_host. Runs usbipd.
  server   Machine that uses the remote device (import side).
           Uses kernel modules: usbip_core, vhci_hcd. Devices appear in lsusb.

SECURITY
  Plain TCP on port 3240 by default — no encryption, no authentication.
  Use only on a trusted LAN or VPN. Do not expose this port to the internet.

REQUIREMENTS
  Linux, root for most operations, packages providing `usbip` and `usbipd`
  (e.g. linux-tools-generic / usbip). Kernel modules must be available.

QUICK START — DIRECT ATTACHMENT (recommended)
  On the client (USB plugged in here), leave this running:

    sudo remote-usb client serve --auto --match 14cd:1212

  On the server (where you want the device), leave this running:

    sudo remote-usb server follow 192.168.1.10 --match 14cd:1212

  Plug or unplug on the client; within a few seconds the server attaches or
  detaches automatically. Prefer --match VID:PID so keyboards/mice stay local.

QUICK START — MANUAL
  Client:
    sudo remote-usb client prepare
    remote-usb client list
    sudo remote-usb client serve
    sudo remote-usb client bind 14cd:1212

  Server:
    sudo remote-usb server prepare
    remote-usb server list 192.168.1.10
    sudo remote-usb server attach 192.168.1.10 14cd:1212
    remote-usb server ports

  Tear down:
    sudo remote-usb server detach 0
    sudo remote-usb client unbind 14cd:1212

MORE HELP
  remote-usb client --help
  remote-usb server --help
  remote-usb client serve --help
  remote-usb server follow --help
";

const TOP_AFTER_HELP: &str = "\
Examples:
  # Direct attachment (auto export + auto attach)
  sudo remote-usb client serve --auto --match 0781:5581
  sudo remote-usb server follow 192.168.1.10 --match 0781:5581

  # Discover devices on the client
  remote-usb client list

  # Manual export / import
  sudo remote-usb client serve
  sudo remote-usb client bind 1-6
  sudo remote-usb server attach 192.168.1.10 1-6

  # Verbose logging
  sudo remote-usb -vv client serve --auto

Environment:
  REMOTE_USB_PORT   Default TCP port (3240)
  RUST_LOG          Log filter (e.g. info, remote_usb=debug)
";

const CLIENT_ABOUT: &str = "Export physical USB devices (run on the machine with the USB port)";

const CLIENT_LONG_ABOUT: &str = "\
Client role — run this on the machine where USB devices are physically plugged in.

The client loads usbip_host, runs the USB/IP export daemon (usbipd), and binds
selected devices so a remote server can attach them.

Typical flow:
  1. remote-usb client list                  # find busid / VID:PID
  2. sudo remote-usb client serve --auto ... # or serve + bind manually
";

const CLIENT_AFTER_HELP: &str = "\
Examples:
  remote-usb client list
  sudo remote-usb client prepare
  sudo remote-usb client serve --auto --match 14cd:1212
  sudo remote-usb client serve --auto --match 14cd:1212 --exclude 046d:c52b
  sudo remote-usb client serve
  sudo remote-usb client bind 1-6
  sudo remote-usb client bind 14cd:1212
  sudo remote-usb client unbind 14cd:1212
";

const SERVER_ABOUT: &str = "Import remote USB devices (run on the machine that will use them)";

const SERVER_LONG_ABOUT: &str = "\
Server role — run this on the machine that should see and use the remote USB device.

The server loads vhci_hcd and attaches devices exported by a client. After attach,
the kernel enumerates the device normally (lsusb, /dev/sdX for mass storage, etc.).

Typical flow:
  1. remote-usb server list <client-host>
  2. sudo remote-usb server follow <client-host> ...   # or attach once
  3. remote-usb server ports
";

const SERVER_AFTER_HELP: &str = "\
Examples:
  sudo remote-usb server prepare
  remote-usb server list 192.168.1.10
  sudo remote-usb server follow 192.168.1.10 --match 14cd:1212
  sudo remote-usb server attach 192.168.1.10 14cd:1212
  remote-usb server ports
  sudo remote-usb server detach 0
";

const SERVE_ABOUT: &str = "Run the USB/IP export daemon (usbipd)";

const SERVE_LONG_ABOUT: &str = "\
Start usbipd so a server can connect and attach exported devices.

Without --auto, you must export devices yourself with `client bind`.
With --auto, matching local devices are bound as they appear (direct attachment).
Pair --auto with `remote-usb server follow <host>` on the server.

WARNING: --auto without --match exports ALL non-hub USB devices on this machine,
including keyboard and mouse (they will disconnect from the client). Prefer:

  sudo remote-usb client serve --auto --match VVVV:PPPP
";

const SERVE_AFTER_HELP: &str = "\
Examples:
  # Manual: daemon only; bind in another terminal
  sudo remote-usb client serve
  sudo remote-usb client bind 14cd:1212

  # Direct attachment: auto-export one flash drive
  sudo remote-usb client serve --auto --match 0781:5581

  # Auto-export several devices; never export a dongle
  sudo remote-usb client serve --auto \\
      --match 14cd:1212 --match 0781:5581 \\
      --exclude 046d:c52b

  # Non-default port and faster scan
  sudo remote-usb client serve --auto --port 3241 --interval 1

  # Leave devices bound after Ctrl+C
  sudo remote-usb client serve --auto --match 14cd:1212 --no-unbind-on-exit
";

const FOLLOW_ABOUT: &str =
    "Continuously attach devices exported by a client (direct attachment)";

const FOLLOW_LONG_ABOUT: &str = "\
Poll a client host and automatically attach every matching exported device.
When a remote device disappears, detach it locally (unless --no-detach-missing).

Pair with `remote-usb client serve --auto` for hands-free plug-and-play.

Mass-storage devices show up under /dev/disk/by-id/ after attach; your desktop
or udisks may mount them automatically.
";

const FOLLOW_AFTER_HELP: &str = "\
Examples:
  # Attach everything the client exports
  sudo remote-usb server follow 192.168.1.10

  # Only a specific device
  sudo remote-usb server follow 192.168.1.10 --match 14cd:1212

  # Custom port / poll interval
  sudo remote-usb server follow client.local --port 3241 --interval 1

  # Keep attachments if the client drops offline briefly
  sudo remote-usb server follow 192.168.1.10 --no-detach-missing
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
    /// Increase logging to stderr (-v info, -vv debug, -vvv trace).
    /// Also honors RUST_LOG (e.g. RUST_LOG=info).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Export physical USB devices (run where the USB is plugged in)
    #[command(
        about = CLIENT_ABOUT,
        long_about = CLIENT_LONG_ABOUT,
        after_help = CLIENT_AFTER_HELP,
        after_long_help = CLIENT_AFTER_HELP,
        arg_required_else_help = true
    )]
    Client {
        #[command(subcommand)]
        command: ClientCmd,
    },
    /// Import remote USB devices (run where you want to use them)
    #[command(
        about = SERVER_ABOUT,
        long_about = SERVER_LONG_ABOUT,
        after_help = SERVER_AFTER_HELP,
        after_long_help = SERVER_AFTER_HELP,
        arg_required_else_help = true
    )]
    Server {
        #[command(subcommand)]
        command: ServerCmd,
    },
}

#[derive(Debug, Subcommand)]
enum ClientCmd {
    /// Load usbip_core and usbip_host kernel modules
    #[command(
        long_about = "Load the kernel modules required to export USB devices \
(usbip_core, usbip_host). Requires root. Safe to re-run if modules are already loaded."
    )]
    Prepare,

    /// List local USB devices (busid, VID:PID, product)
    #[command(
        long_about = "List USB devices visible on this machine via `usbip list --local`.\n\n\
Use the BUSID (e.g. 1-6) or VID:PID (e.g. 14cd:1212) with bind/unbind.\n\
Does not require root.",
        after_help = "Examples:\n  remote-usb client list\n  remote-usb -v client list"
    )]
    List,

    /// Export a device so the server can attach it
    #[command(
        long_about = "Bind a local USB device to usbip-host so it can be exported.\n\n\
Requires root and a running export daemon (`client serve`).\n\
SELECTOR may be a busid (1-6) or VID:PID (14cd:1212). VID:PID must be unique.",
        after_help = "Examples:\n  \
sudo remote-usb client bind 1-6\n  \
sudo remote-usb client bind 14cd:1212\n  \
sudo remote-usb client bind 0781:5581"
    )]
    Bind {
        /// Device busid (e.g. 1-6) or VID:PID (e.g. 14cd:1212)
        selector: String,
    },

    /// Stop exporting a device (return it to local use)
    #[command(
        long_about = "Unbind a device from usbip-host so it is no longer exportable \
and can be used again by local drivers.\n\nRequires root.",
        after_help = "Examples:\n  \
sudo remote-usb client unbind 1-6\n  \
sudo remote-usb client unbind 14cd:1212"
    )]
    Unbind {
        /// Device busid (e.g. 1-6) or VID:PID (e.g. 14cd:1212)
        selector: String,
    },

    /// Run the USB/IP export daemon (usbipd)
    #[command(
        about = SERVE_ABOUT,
        long_about = SERVE_LONG_ABOUT,
        after_help = SERVE_AFTER_HELP,
        after_long_help = SERVE_AFTER_HELP
    )]
    Serve {
        /// TCP listen port for USB/IP (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,
        /// Listen on IPv4 only
        #[arg(long)]
        ipv4: bool,
        /// Listen on IPv6 only
        #[arg(long)]
        ipv6: bool,
        /// Auto-export matching devices as they appear (direct attachment).
        /// Pair with `server follow`. Prefer --match to avoid exporting keyboards.
        #[arg(long)]
        auto: bool,
        /// Only auto-export these VID:PID values (repeatable).
        /// If omitted with --auto, all non-hub devices are candidates.
        #[arg(long = "match", value_name = "VID:PID")]
        match_ids: Vec<String>,
        /// Never auto-export these VID:PID values (repeatable)
        #[arg(long = "exclude", value_name = "VID:PID")]
        exclude_ids: Vec<String>,
        /// Also auto-export USB hubs (skipped by default)
        #[arg(long)]
        include_hubs: bool,
        /// Seconds between device scans when --auto is set
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: f32,
        /// Do not unbind auto-exported devices on exit (default: unbind them)
        #[arg(long)]
        no_unbind_on_exit: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ServerCmd {
    /// Load usbip_core and vhci_hcd kernel modules
    #[command(
        long_about = "Load the kernel modules required to import remote USB devices \
(usbip_core, vhci_hcd). Requires root. Safe to re-run if modules are already loaded."
    )]
    Prepare,

    /// List devices currently exported by a remote client
    #[command(
        long_about = "Query a client host for devices it has bound/exported.\n\n\
The client must be running `client serve` (and have bound devices, or use --auto).",
        after_help = "Examples:\n  \
remote-usb server list 192.168.1.10\n  \
remote-usb server list client.local --port 3241"
    )]
    List {
        /// Client hostname or IP address
        host: String,
        /// TCP port on the client (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,
    },

    /// Attach one remote device so it appears as a local USB device
    #[command(
        long_about = "Attach a single device exported by the client. After success, \
check `lsusb` and `remote-usb server ports`.\n\n\
SELECTOR is a busid or VID:PID as shown by `server list`.\nRequires root.",
        after_help = "Examples:\n  \
sudo remote-usb server attach 192.168.1.10 1-6\n  \
sudo remote-usb server attach 192.168.1.10 14cd:1212\n  \
sudo remote-usb server attach client.local 0781:5581 --port 3240"
    )]
    Attach {
        /// Client hostname or IP address
        host: String,
        /// Device busid or VID:PID on the client
        selector: String,
        /// TCP port on the client (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,
    },

    /// Detach an imported device by VHCI port number
    #[command(
        long_about = "Detach a previously attached remote device.\n\n\
PORT is the VHCI port from `remote-usb server ports` (e.g. 0, 1), not the TCP port.\n\
Requires root.",
        after_help = "Examples:\n  \
remote-usb server ports\n  \
sudo remote-usb server detach 0"
    )]
    Detach {
        /// VHCI port number from `server ports` (not the TCP port)
        port: u32,
    },

    /// List imported (attached) remote USB devices and VHCI ports
    #[command(
        long_about = "Show devices currently attached via vhci_hcd, including local \
port numbers used by `server detach`.",
        after_help = "Examples:\n  remote-usb server ports"
    )]
    Ports,

    /// Continuously attach devices exported by a client
    #[command(
        about = FOLLOW_ABOUT,
        long_about = FOLLOW_LONG_ABOUT,
        after_help = FOLLOW_AFTER_HELP,
        after_long_help = FOLLOW_AFTER_HELP
    )]
    Follow {
        /// Client hostname or IP address
        host: String,
        /// TCP port on the client (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,
        /// Only auto-attach these VID:PID values (repeatable).
        /// Default: all devices the client has exported.
        #[arg(long = "match", value_name = "VID:PID")]
        match_ids: Vec<String>,
        /// Never auto-attach these VID:PID values (repeatable)
        #[arg(long = "exclude", value_name = "VID:PID")]
        exclude_ids: Vec<String>,
        /// Also attach USB hubs (skipped by default)
        #[arg(long)]
        include_hubs: bool,
        /// Seconds between polls of the client
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: f32,
        /// Do not detach when a remote device disappears (default: detach)
        #[arg(long)]
        no_detach_missing: bool,
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
        Commands::Client { command } => match command {
            ClientCmd::Prepare => client::prepare(),
            ClientCmd::List => client::list(),
            ClientCmd::Bind { selector } => client::bind(&selector),
            ClientCmd::Unbind { selector } => client::unbind(&selector),
            ClientCmd::Serve {
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
                let filter = DeviceFilter::from_cli(&match_ids, &exclude_ids, include_hubs)?;
                client::serve(client::ServeOptions {
                    port,
                    ipv4_only: ipv4,
                    ipv6_only: ipv6,
                    auto,
                    filter,
                    interval: Duration::from_secs_f32(interval),
                    unbind_on_exit: !no_unbind_on_exit,
                })
            }
        },
        Commands::Server { command } => match command {
            ServerCmd::Prepare => server::prepare(),
            ServerCmd::List { host, port } => server::list(&host, port),
            ServerCmd::Attach {
                host,
                selector,
                port,
            } => server::attach(&host, &selector, port),
            ServerCmd::Detach { port } => server::detach(port),
            ServerCmd::Ports => server::ports(),
            ServerCmd::Follow {
                host,
                port,
                match_ids,
                exclude_ids,
                include_hubs,
                interval,
                no_detach_missing,
            } => {
                if interval <= 0.0 {
                    return Err(Error::Message("--interval must be positive".into()));
                }
                let filter = DeviceFilter::from_cli(&match_ids, &exclude_ids, include_hubs)?;
                server::follow(server::FollowOptions {
                    host,
                    port,
                    filter,
                    interval: Duration::from_secs_f32(interval),
                    detach_missing: !no_detach_missing,
                })
            }
        },
    }
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
