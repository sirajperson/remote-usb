//! remote-usb — clients share USB devices with a server over Linux USB/IP.
//!
//! Product model:
//! - **Client** (default): machine with physical USB; shares devices with a server.
//! - **Server**: waits to use devices that clients share with it.
//!
//! USB/IP note: the client runs the export listener (`share`); the server connects
//! to each client and attaches devices.

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
    "Clients share USB devices with a server over the network (Linux USB/IP)";

const TOP_LONG_ABOUT: &str = "\
The server imports USB devices from clients over a trusted network (Linux USB/IP).

MENTAL MODEL
  SERVER  — imports devices from clients; after import, devices show in lsusb.
  CLIENT  — physical USB is plugged in; exports devices to the server.

  Default commands (list, bind, share) are for the CLIENT.
  `remote-usb server …` is for the SERVER.

SECURITY
  Plain TCP (default port 3240), no encryption or authentication.
  Trusted LAN/VPN only. Do not expose the port to the internet.

QUICK START — DIRECT ATTACHMENT
  1) SERVER (imports from the client):

    sudo remote-usb server prepare
    sudo remote-usb server --client 192.168.1.10 --auto --match 14cd:1212

  2) CLIENT (exports its USB):

    sudo remote-usb share --auto --match 14cd:1212

  3) On the SERVER, confirm:

    lsusb
    remote-usb ports

QUICK START — MANUAL
  1) Server loads:

    sudo remote-usb server prepare

  2) Client exports:

    remote-usb list
    sudo remote-usb share 0.0.0.0
    sudo remote-usb bind 1-6

  3) Server imports from the client:

    remote-usb server --client 192.168.1.10 list
    sudo remote-usb server --client 192.168.1.10 bind 1-6
    lsusb
    remote-usb ports
";

const TOP_AFTER_HELP: &str = "\
Examples:
  # SERVER — import devices from a client
  sudo remote-usb server prepare
  remote-usb server --client 192.168.1.10 list
  sudo remote-usb server --client 192.168.1.10 bind 1-6
  sudo remote-usb server --client 192.168.1.10 --auto --match 0781:5581
  lsusb
  remote-usb ports
  sudo remote-usb detach 0

  # CLIENT — export local USB to the server
  remote-usb list
  sudo remote-usb share --auto --match 0781:5581
  sudo remote-usb share 0.0.0.0
  sudo remote-usb bind 1-6
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
    /// List local USB devices (client)
    #[command(
        long_about = "List USB devices plugged into this client machine.\n\n\
Use BUSID (e.g. 1-6) or VID:PID (e.g. 14cd:1212) with bind/unbind.",
        after_help = "Examples:\n  remote-usb list"
    )]
    List,

    /// Export a local USB device to the server (client)
    #[command(
        long_about = "Export a local USB device so the server can import it.\n\n\
Requires root and a running `remote-usb share` on this client.\n\
SELECTOR: busid (1-6) or VID:PID (14cd:1212).",
        after_help = "Examples:\n  \
sudo remote-usb bind 1-6\n  \
sudo remote-usb bind 14cd:1212"
    )]
    Bind {
        /// Busid (1-6) or VID:PID (14cd:1212)
        selector: String,
    },

    /// Stop exporting a local USB device (client)
    #[command(
        long_about = "Stop exporting a device so local drivers can use it again.",
        after_help = "Examples:\n  sudo remote-usb unbind 1-6"
    )]
    Unbind {
        /// Busid or VID:PID
        selector: String,
    },

    /// Load client kernel modules (usbip_host)
    #[command(
        long_about = "Load usbip_core + usbip_host on this client so devices can be exported."
    )]
    Prepare,

    /// Client: export this machine's USB so the server can import it
    #[command(
        about = "Client: export this machine's USB to a server",
        long_about = "\
CLIENT command — run on the machine where USB is plugged in.

Exports devices so a remote server can import them.
BIND_ADDR defaults to 0.0.0.0 (all interfaces). Restrict access with a firewall.

Without --auto, export devices with `remote-usb bind`.
With --auto, matching devices are exported as they appear.

Typical order:
  1. On server:  sudo remote-usb server prepare
  2. On client:  sudo remote-usb share …  and  bind …
  3. On server:  remote-usb server --client <this-ip> bind …  then  lsusb

WARNING: --auto without --match exports ALL non-hub USB devices (keyboard/mouse too).",
        after_help = "\
Examples:
  sudo remote-usb share
  sudo remote-usb share 0.0.0.0
  sudo remote-usb share --auto --match 0781:5581
  sudo remote-usb bind 14cd:1212
"
    )]
    Share {
        /// Listen intent (default 0.0.0.0 = all interfaces)
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

    /// Server: import USB devices that clients export
    #[command(
        about = "Server: import USB devices from clients",
        long_about = "\
SERVER command — run on the machine that imports USB from clients.

Flow:
  1. Server loads:  sudo remote-usb server prepare
  2. Client exports devices (`share` + `bind` on the client)
  3. Server imports:  server --client <CLIENT_IP> bind …
  4. Confirm on server:  lsusb   and   remote-usb ports

  sudo remote-usb server prepare
  remote-usb server --client <CLIENT_IP> list
  sudo remote-usb server --client <CLIENT_IP> bind <device>
  sudo remote-usb server --client <CLIENT_IP> --auto
  lsusb

Optional BIND_ADDR (default 0.0.0.0) documents this server.",
        after_help = "\
Examples:
  # Load server, then import from a client
  sudo remote-usb server prepare
  remote-usb server --client 192.168.1.10 list
  sudo remote-usb server --client 192.168.1.10 bind 1-6
  sudo remote-usb server --client 192.168.1.10 bind 14cd:1212
  sudo remote-usb server 0.0.0.0 --client 192.168.1.10 --auto --match 14cd:1212
  lsusb
  remote-usb ports
  sudo remote-usb detach 0
",
        arg_required_else_help = true
    )]
    Server {
        /// This server's address note (default 0.0.0.0)
        #[arg(default_value = DEFAULT_BIND)]
        bind_addr: String,

        /// Client IP/hostname that is sharing USB (required for list/bind/--auto)
        #[arg(long = "client", value_name = "CLIENT_IP")]
        client: Option<String>,

        /// TCP port on the client (default 3240)
        #[arg(long, default_value_t = DEFAULT_PORT, env = "REMOTE_USB_PORT")]
        port: u16,

        /// Continuously attach devices the client shares
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

        /// Do not detach when a client device disappears
        #[arg(long)]
        no_detach_missing: bool,

        #[command(subcommand)]
        command: Option<ServerCmd>,
    },

    /// List devices imported on this server (also check lsusb)
    #[command(
        long_about = "Show devices currently imported on this server from clients.\n\
After import, devices also appear in `lsusb`.\n\
Port numbers are used with `detach`.",
        after_help = "Examples:\n  remote-usb ports\n  lsusb"
    )]
    Ports,

    /// Stop importing a client device on this server
    #[command(
        long_about = "Detach a device previously imported from a client.\n\
PORT is the VHCI port from `remote-usb ports` (0, 1, …), not the TCP port.",
        after_help = "Examples:\n  remote-usb ports\n  sudo remote-usb detach 0"
    )]
    Detach {
        /// VHCI port from `remote-usb ports`
        port: u32,
    },
}

#[derive(Debug, Subcommand)]
enum ServerCmd {
    /// Load server kernel modules (vhci_hcd) — do this first
    #[command(
        long_about = "Load usbip_core + vhci_hcd on this server so it can import client devices.\n\
Typically the first step on the server before clients export."
    )]
    Prepare,

    /// List devices a client is currently exporting
    #[command(
        long_about = "List USB devices a client is exporting (via `remote-usb share` + bind).",
        after_help = "Examples:\n  remote-usb server --client 192.168.1.10 list"
    )]
    List,

    /// Import one device from a client onto this server
    #[command(
        name = "bind",
        visible_alias = "attach",
        long_about = "Import one device from a client so it appears as local USB on the server.\n\n\
After success, confirm with `lsusb` and `remote-usb ports`.\n\
SELECTOR: busid or VID:PID from `server --client <ip> list`.",
        after_help = "Examples:\n  \
sudo remote-usb server --client 192.168.1.10 bind 1-6\n  \
sudo remote-usb server --client 192.168.1.10 bind 14cd:1212\n  \
lsusb"
    )]
    Bind {
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
        Commands::Bind { selector } => client::bind(&selector),
        Commands::Unbind { selector } => client::unbind(&selector),
        Commands::Prepare => client::prepare(),
        Commands::Ports => server::ports(),
        Commands::Detach { port } => server::detach(port),

        Commands::Share {
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

        Commands::Server {
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
        } => run_server(
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
    }
}

fn run_server(
    bind_addr: &str,
    client: Option<&str>,
    port: u16,
    auto: bool,
    match_ids: &[String],
    exclude_ids: &[String],
    include_hubs: bool,
    interval: f32,
    no_detach_missing: bool,
    command: Option<ServerCmd>,
) -> Result<()> {
    let _ = bind_addr; // product-facing; USB/IP attaches outbound to clients

    if matches!(command, Some(ServerCmd::Prepare)) {
        return server::prepare();
    }

    if auto && command.is_some() {
        return Err(Error::Message(
            "use either --auto or a subcommand (list/bind), not both".into(),
        ));
    }

    let client = client.ok_or_else(|| {
        Error::Message(
            "server needs --client <CLIENT_IP> (except for `server prepare`)\n\
             example: remote-usb server --client 192.168.1.10 list\n\
             example: remote-usb server --client 192.168.1.10 --auto"
                .into(),
        )
    })?;

    if auto {
        if interval <= 0.0 {
            return Err(Error::Message("--interval must be positive".into()));
        }
        let filter = DeviceFilter::from_cli(match_ids, exclude_ids, include_hubs)?;
        return server::follow(server::FollowOptions {
            host: client.to_string(),
            port,
            filter,
            interval: Duration::from_secs_f32(interval),
            detach_missing: !no_detach_missing,
        });
    }

    match command {
        Some(ServerCmd::List) => server::list(client, port),
        Some(ServerCmd::Bind { selector }) => server::attach(client, &selector, port),
        Some(ServerCmd::Prepare) => server::prepare(), // already handled; keep exhaustive
        None => Err(Error::Message(
            "specify list, bind, or --auto\n\
             try: remote-usb server --help"
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
