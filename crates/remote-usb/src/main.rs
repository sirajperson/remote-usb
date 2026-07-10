//! remote-usb — server receives USB devices that clients attach to it.
//!
//! Direction (do not invert):
//! - **Server** runs `serve` and uses USB devices from clients.
//! - **Client** has the physical USB and **attaches** devices to the server.

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

const TOP_ABOUT: &str =
    "Server receives USB devices; clients attach their devices to the server";

const TOP_LONG_ABOUT: &str = "\
remote-usb: the server receives USB devices that clients attach to it.

DIRECTION (fixed — do not reverse these roles)
  SERVER  runs `serve` and ends up with the devices (see them in lsusb).
  CLIENT  has the physical USB plug and attaches devices to the server.

  sudo remote-usb serve 0.0.0.0 --client <CLIENT_IP> --auto
  sudo remote-usb attach <SERVER_IP> --auto --match <VID:PID>

SECURITY
  Plain TCP port 3240 by default. No encryption or auth.
  Trusted LAN/VPN only.

MANUAL FLOW
  1) Server:

       sudo remote-usb serve 0.0.0.0 --client 192.168.1.10

     (add --auto to keep importing; or import one device later)

  2) Client (USB plugged in here):

       remote-usb list
       sudo remote-usb attach 192.168.1.20 1-6

     (192.168.1.20 = server IP)

  3) On the server:

       lsusb
       remote-usb ports
";

const TOP_AFTER_HELP: &str = "\
Examples:
  # SERVER — receive devices from a client
  sudo remote-usb serve 0.0.0.0 --client 192.168.1.10 --auto --match 14cd:1212
  sudo remote-usb serve --client 192.168.1.10 list
  sudo remote-usb serve --client 192.168.1.10 import 1-6
  lsusb
  remote-usb ports
  sudo remote-usb detach 0

  # CLIENT — attach local USB to the server
  remote-usb list
  sudo remote-usb attach 192.168.1.20 1-6
  sudo remote-usb attach 192.168.1.20 --auto --match 14cd:1212
  sudo remote-usb unbind 1-6

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
    // ----- SERVER -----
    /// SERVER: wait for clients and receive the devices they attach
    #[command(
        about = "SERVER: run the server (receive devices from clients)",
        long_about = "\
SERVER command — run this on the machine that should have the USB devices.

The server does NOT plug in the physical USB. Clients do, and they attach
those devices to this server.

  sudo remote-usb serve 0.0.0.0 --client <CLIENT_IP> --auto
  sudo remote-usb serve --client <CLIENT_IP> list
  sudo remote-usb serve --client <CLIENT_IP> import <device>

After a device is received, confirm on this machine:

  lsusb
  remote-usb ports

--client is the IP of the machine where the USB is physically plugged in.",
        after_help = "\
Examples:
  sudo remote-usb serve 0.0.0.0 --client 192.168.1.10 --auto
  sudo remote-usb serve --client 192.168.1.10 list
  sudo remote-usb serve --client 192.168.1.10 import 14cd:1212
  lsusb
"
    )]
    Serve {
        /// Address this server listens for / binds (default 0.0.0.0 = all interfaces)
        #[arg(default_value = DEFAULT_BIND)]
        bind_addr: String,

        /// Client IP — machine that has the physical USB and will attach devices
        #[arg(long = "client", value_name = "CLIENT_IP")]
        client: Option<String>,

        /// TCP port used with the client (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,

        /// Keep receiving every device the client attaches
        #[arg(long)]
        auto: bool,

        /// Only auto-receive these VID:PID values (repeatable)
        #[arg(long = "match", value_name = "VID:PID")]
        match_ids: Vec<String>,

        /// Never auto-receive these VID:PID values
        #[arg(long = "exclude", value_name = "VID:PID")]
        exclude_ids: Vec<String>,

        /// Also receive USB hubs
        #[arg(long)]
        include_hubs: bool,

        /// Seconds between polls when --auto is set
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: f32,

        /// Keep devices if the client disconnects (default: remove them)
        #[arg(long)]
        no_detach_missing: bool,

        #[command(subcommand)]
        command: Option<ServeCmd>,
    },

    /// SERVER: list devices already received from clients
    #[command(
        long_about = "List USB devices this server has already received from clients.\n\
Also run `lsusb` — received devices appear as normal local USB.",
        after_help = "Examples:\n  remote-usb ports\n  lsusb"
    )]
    Ports,

    /// SERVER: remove a received device
    #[command(
        long_about = "Remove a device previously received from a client.\n\
PORT is from `remote-usb ports` (0, 1, …), not the TCP port.",
        after_help = "Examples:\n  remote-usb ports\n  sudo remote-usb detach 0"
    )]
    Detach {
        /// VHCI port from `remote-usb ports`
        port: u32,
    },

    // ----- CLIENT -----
    /// CLIENT: list USB devices plugged into this machine
    #[command(
        long_about = "List local USB devices on this client (where the stick is plugged in).",
        after_help = "Examples:\n  remote-usb list"
    )]
    List,

    /// CLIENT: attach a local USB device to the server
    #[command(
        about = "CLIENT: attach a local USB device to the server",
        long_about = "\
CLIENT command — run on the machine where the USB is physically plugged in.

Attaches local device(s) so the server can receive them.

  remote-usb list
  sudo remote-usb attach <SERVER_IP> 1-6
  sudo remote-usb attach <SERVER_IP> --auto --match 14cd:1212

SERVER_IP is the machine running `remote-usb serve`.

The server should already be running, e.g.:

  sudo remote-usb serve 0.0.0.0 --client <this-client-ip> --auto

WARNING: --auto without --match may attach ALL non-hub devices (keyboard/mouse).",
        after_help = "\
Examples:
  sudo remote-usb attach 192.168.1.20 1-6
  sudo remote-usb attach 192.168.1.20 14cd:1212
  sudo remote-usb attach 192.168.1.20 --auto --match 14cd:1212
"
    )]
    Attach {
        /// IP or hostname of the server (machine running `serve`)
        server: String,

        /// Local busid or VID:PID to attach (omit with --auto)
        selector: Option<String>,

        /// Attach matching devices automatically as they appear
        #[arg(long)]
        auto: bool,

        /// TCP port (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,

        /// IPv4 only for the client export listener
        #[arg(long)]
        ipv4: bool,

        /// IPv6 only for the client export listener
        #[arg(long)]
        ipv6: bool,

        /// Only auto-attach these VID:PID values (repeatable)
        #[arg(long = "match", value_name = "VID:PID")]
        match_ids: Vec<String>,

        /// Never auto-attach these VID:PID values
        #[arg(long = "exclude", value_name = "VID:PID")]
        exclude_ids: Vec<String>,

        /// Also attach USB hubs
        #[arg(long)]
        include_hubs: bool,

        /// Seconds between scans when --auto is set
        #[arg(long, default_value_t = DEFAULT_INTERVAL_SECS)]
        interval: f32,

        /// Keep devices attached after exit (default: release on exit with --auto)
        #[arg(long)]
        no_unbind_on_exit: bool,
    },

    /// CLIENT: stop attaching a local device (return it to local use)
    #[command(
        long_about = "Stop offering a local device to the server; restore local drivers.",
        after_help = "Examples:\n  sudo remote-usb unbind 1-6"
    )]
    Unbind {
        /// Busid or VID:PID
        selector: String,
    },

    /// CLIENT: load modules needed to attach devices to a server
    #[command(
        long_about = "Load usbip_core + usbip_host on the client (physical USB machine)."
    )]
    Prepare,
}

#[derive(Debug, Subcommand)]
enum ServeCmd {
    /// Load server modules (vhci) — also done automatically by serve
    #[command(long_about = "Load usbip_core + vhci_hcd on the server.")]
    Prepare,

    /// List devices a client is currently offering
    #[command(
        long_about = "List devices the client has attached for this server.",
        after_help = "Examples:\n  remote-usb serve --client 192.168.1.10 list"
    )]
    List,

    /// Receive one device from the client onto this server
    #[command(
        long_about = "Receive one device from --client so it appears in lsusb on the server.",
        after_help = "Examples:\n  \
sudo remote-usb serve --client 192.168.1.10 import 1-6\n  \
lsusb"
    )]
    Import {
        /// Busid or VID:PID on the client
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
        Commands::Prepare => client::prepare(),
        Commands::Unbind { selector } => client::unbind(&selector),
        Commands::Ports => server::ports(),
        Commands::Detach { port } => server::detach(port),

        Commands::Serve {
            bind_addr,
            client,
            port,
            auto,
            match_ids,
            exclude_ids,
            include_hubs,
            interval,
            no_detach_missing,
            command,
        } => run_serve(
            &bind_addr,
            client.as_deref(),
            port,
            auto,
            &match_ids,
            &exclude_ids,
            include_hubs,
            interval,
            no_detach_missing,
            command,
        ),

        Commands::Attach {
            server,
            selector,
            auto,
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
                    "specify a device (busid or VID:PID) or pass --auto\n\
                     example: remote-usb attach 192.168.1.20 1-6\n\
                     example: remote-usb attach 192.168.1.20 --auto --match 14cd:1212"
                        .into(),
                ));
            }
            if interval <= 0.0 {
                return Err(Error::Message("--interval must be positive".into()));
            }
            let filter = DeviceFilter::from_cli(&match_ids, &exclude_ids, include_hubs)?;
            client::attach_to_server(client::AttachOptions {
                server_addr: server,
                port,
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

fn run_serve(
    bind_addr: &str,
    client: Option<&str>,
    port: u16,
    auto: bool,
    match_ids: &[String],
    exclude_ids: &[String],
    include_hubs: bool,
    interval: f32,
    no_detach_missing: bool,
    command: Option<ServeCmd>,
) -> Result<()> {
    match command {
        Some(ServeCmd::Prepare) => return server::prepare(),
        Some(ServeCmd::List) | Some(ServeCmd::Import { .. }) | None => {}
    }

    if auto && command.is_some() {
        return Err(Error::Message(
            "use either --auto or a subcommand (list/import), not both".into(),
        ));
    }

    // Bare `serve 0.0.0.0` without --client: prepare and explain next step.
    if client.is_none() && command.is_none() && !auto {
        server::prepare()?;
        println!(
            "Server ready on {bind_addr} (import side loaded).\n\
             \n\
             This machine RECEIVES USB devices from clients. It does not export USB.\n\
             \n\
             Next:\n\
               1. On each CLIENT (USB plugged in):\n\
                    sudo remote-usb attach <this-server-ip> <device>\n\
                    # or: sudo remote-usb attach <this-server-ip> --auto --match VID:PID\n\
               2. On this SERVER, receive them:\n\
                    sudo remote-usb serve {bind_addr} --client <CLIENT_IP> --auto\n\
                    # or once: sudo remote-usb serve --client <CLIENT_IP> import <device>\n\
               3. Confirm here:\n\
                    lsusb\n\
                    remote-usb ports"
        );
        return Ok(());
    }

    let client = client.ok_or_else(|| {
        Error::Message(
            "serve needs --client <CLIENT_IP> (the machine with the physical USB)\n\
             example: remote-usb serve 0.0.0.0 --client 192.168.1.10 --auto\n\
             example: remote-usb serve --client 192.168.1.10 list\n\
             example: remote-usb serve --client 192.168.1.10 import 1-6"
                .into(),
        )
    })?;

    if auto {
        if interval <= 0.0 {
            return Err(Error::Message("--interval must be positive".into()));
        }
        let filter = DeviceFilter::from_cli(match_ids, exclude_ids, include_hubs)?;
        println!(
            "Server listening on {bind_addr}; receiving devices from client {client}.\n\
             Clients attach devices with: remote-usb attach <this-server-ip> …"
        );
        return server::follow(server::FollowOptions {
            host: client.to_string(),
            port,
            filter,
            interval: Duration::from_secs_f32(interval),
            detach_missing: !no_detach_missing,
        });
    }

    match command {
        Some(ServeCmd::List) => server::list(client, port),
        Some(ServeCmd::Import { selector }) => server::attach(client, &selector, port),
        Some(ServeCmd::Prepare) => server::prepare(),
        None => Err(Error::Message(
            "specify --auto, list, or import\n\
             example: remote-usb serve --client 192.168.1.10 --auto\n\
             example: remote-usb serve --client 192.168.1.10 import 1-6"
                .into(),
        )),
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
