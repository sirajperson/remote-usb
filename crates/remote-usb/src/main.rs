//! remote-usb — classic client/server USB sharing.
//!
//! - **Server** runs `serve` and **waits**. It never needs a client IP up front.
//! - **Client** runs `attach <server>` and **connects** to offer local USB devices.
//!
//! The server learns each client from the incoming control connection.

mod client;
mod control;
mod error;
mod filter;
mod kmod;
mod privilege;
mod server;
mod usbip_cmd;

use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::control::DEFAULT_CONTROL_PORT;
use crate::error::{Error, Result};
use crate::filter::DeviceFilter;

const DEFAULT_USBIP_PORT: u16 = 3240;
const DEFAULT_INTERVAL_SECS: f32 = 2.0;
const DEFAULT_BIND: &str = "0.0.0.0";

const TOP_ABOUT: &str =
    "Server waits for clients; clients attach their USB devices to the server";

const TOP_LONG_ABOUT: &str = "\
Classic client/server USB sharing over a trusted network.

ARCHITECTURE
  SERVER  runs first and waits:

    sudo remote-usb serve 0.0.0.0

  CLIENT  has the physical USB and attaches devices to that server:

    sudo remote-usb attach <SERVER_IP> 1-6
    sudo remote-usb attach <SERVER_IP> --auto --match 14cd:1212

  The server does not need to know client IPs in advance. Clients connect
  to the server; the server receives whatever they attach.

  On the server after a client attaches:

    lsusb
    remote-usb ports

SECURITY
  Plain TCP. Default control port 3250; USB/IP data on 3240.
  Trusted LAN/VPN only. No authentication.
";

const TOP_AFTER_HELP: &str = "\
Examples:
  # Terminal A — SERVER (waits)
  sudo remote-usb serve 0.0.0.0

  # Terminal B — CLIENT (USB plugged in here)
  remote-usb list
  sudo remote-usb attach 192.168.1.20 1-6

  # Back on SERVER
  lsusb
  remote-usb ports

  # Auto: client offers matching devices as they appear
  sudo remote-usb attach 192.168.1.20 --auto --match 14cd:1212

Environment:
  REMOTE_USB_PORT           USB/IP data port (3240)
  REMOTE_USB_CONTROL_PORT   Control port (3250)
  RUST_LOG                  Log filter
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
    /// More logging (-v info, -vv debug).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// SERVER: listen and wait for clients to attach devices
    #[command(
        about = "SERVER: wait for clients to attach USB devices",
        long_about = "\
SERVER command — start this first. The server sits and waits.

Clients then connect and attach their devices:

  sudo remote-usb serve 0.0.0.0
  # on client: sudo remote-usb attach <this-server-ip> 1-6
  # on server: lsusb

No --client flag. You do not preconfigure client addresses.
Clients introduce themselves when they connect.",
        after_help = "\
Examples:
  sudo remote-usb serve
  sudo remote-usb serve 0.0.0.0
  sudo remote-usb serve 0.0.0.0 --control-port 3250
  remote-usb serve prepare
"
    )]
    Serve {
        /// Listen address (default 0.0.0.0 = all interfaces)
        #[arg(default_value = DEFAULT_BIND)]
        bind_addr: String,

        /// Control port clients connect to (default 3250)
        #[arg(
            long,
            default_value_t = DEFAULT_CONTROL_PORT,
            env = "REMOTE_USB_CONTROL_PORT"
        )]
        control_port: u16,

        #[command(subcommand)]
        command: Option<ServeCmd>,
    },

    /// SERVER: list devices already received
    #[command(after_help = "Examples:\n  remote-usb ports\n  lsusb")]
    Ports,

    /// SERVER: remove a received device
    #[command(after_help = "Examples:\n  sudo remote-usb detach 0")]
    Detach {
        /// VHCI port from `remote-usb ports`
        port: u32,
    },

    /// CLIENT: list local USB devices
    #[command(after_help = "Examples:\n  remote-usb list")]
    List,

    /// CLIENT: attach a local USB device to a waiting server
    #[command(
        about = "CLIENT: attach local USB to a server that is running serve",
        long_about = "\
CLIENT command — physical USB is on this machine.

The server must already be waiting (`remote-usb serve`).
This client connects to the server and attaches device(s).

  sudo remote-usb attach <SERVER_IP> 1-6
  sudo remote-usb attach <SERVER_IP> --auto --match 14cd:1212

Keep the process running while the server uses the device.
Ctrl+C revokes the device from the server.",
        after_help = "\
Examples:
  sudo remote-usb attach 192.168.1.20 1-6
  sudo remote-usb attach 192.168.1.20 14cd:1212
  sudo remote-usb attach 192.168.1.20 --auto --match 14cd:1212
"
    )]
    Attach {
        /// Server IP/hostname (machine running `serve`)
        server: String,

        /// Local busid or VID:PID (omit when using --auto)
        selector: Option<String>,

        /// Attach matching devices as they appear
        #[arg(long)]
        auto: bool,

        /// Control port on the server (default 3250)
        #[arg(
            long,
            default_value_t = DEFAULT_CONTROL_PORT,
            env = "REMOTE_USB_CONTROL_PORT"
        )]
        control_port: u16,

        /// USB/IP data port on this client (default 3240)
        #[arg(long, default_value_t = DEFAULT_USBIP_PORT, env = "REMOTE_USB_PORT")]
        port: u16,

        #[arg(long)]
        ipv4: bool,

        #[arg(long)]
        ipv6: bool,

        #[arg(long = "match", value_name = "VID:PID")]
        match_ids: Vec<String>,

        #[arg(long = "exclude", value_name = "VID:PID")]
        exclude_ids: Vec<String>,

        #[arg(long)]
        include_hubs: bool,

        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: f32,

        #[arg(long)]
        no_unbind_on_exit: bool,
    },

    /// CLIENT: release a local device
    #[command(after_help = "Examples:\n  sudo remote-usb unbind 1-6")]
    Unbind {
        selector: String,
    },

    /// CLIENT: load client kernel modules
    Prepare,
}

#[derive(Debug, Subcommand)]
enum ServeCmd {
    /// Load server kernel modules only
    Prepare,
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
        Commands::Prepare => client::prepare(),
        Commands::Unbind { selector } => client::unbind(&selector),
        Commands::Ports => server::ports(),
        Commands::Detach { port } => server::detach(port),

        Commands::Serve {
            bind_addr,
            control_port,
            command,
        } => match command {
            Some(ServeCmd::Prepare) => server::prepare(),
            None => control::run_server(control::ServeOptions {
                bind_addr,
                control_port,
            }),
        },

        Commands::Attach {
            server,
            selector,
            auto,
            control_port,
            port,
            ipv4,
            ipv6,
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
            if auto && selector.is_some() {
                return Err(Error::Message(
                    "use either a device selector or --auto, not both".into(),
                ));
            }
            if !auto && selector.is_none() {
                return Err(Error::Message(
                    "specify a device or --auto\n\
                     example: remote-usb attach 192.168.1.20 1-6"
                        .into(),
                ));
            }
            if interval <= 0.0 {
                return Err(Error::Message("--interval must be positive".into()));
            }
            let filter = DeviceFilter::from_cli(&match_ids, &exclude_ids, include_hubs)?;
            client::attach_to_server(client::AttachOptions {
                server_addr: server,
                control_port,
                usbip_port: port,
                ipv4_only: ipv4,
                ipv6_only: ipv6,
                selector,
                auto,
                filter,
                interval: Duration::from_secs_f32(interval),
                unbind_on_exit: !no_unbind_on_exit,
            })
        }
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
