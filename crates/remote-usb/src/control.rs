//! Control-plane protocol: the server listens; clients connect and offer devices.
//!
//! After an OFFER, the server runs USB/IP attach back to the client (LAN).
//! Client identity is taken from the TCP peer address — the server never needs
//! a preconfigured client IP.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::kmod::ensure_import_modules;
use crate::privilege::require_root;
use crate::usbip_cmd;

/// Default control port (USB/IP data plane stays on 3240).
pub const DEFAULT_CONTROL_PORT: u16 = 3250;

// ---------------------------------------------------------------------------
// Wire format (line-based, UTF-8)
//
// Client → Server:
//   OFFER <usbip_port> <busid>\n
//   REVOKE <busid>\n
//   PING\n
//
// Server → Client:
//   OK\n
//   ERR <message>\n
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Offer { usbip_port: u16, busid: String },
    Revoke { busid: String },
    Ping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Ok,
    Err(String),
}

pub fn parse_client_msg(line: &str) -> Result<ClientMsg> {
    let line = line.trim();
    let mut parts = line.split_whitespace();
    let cmd = parts
        .next()
        .ok_or_else(|| Error::Message("empty control message".into()))?;
    match cmd.to_ascii_uppercase().as_str() {
        "OFFER" => {
            let port_s = parts
                .next()
                .ok_or_else(|| Error::Message("OFFER missing usbip port".into()))?;
            let busid = parts
                .next()
                .ok_or_else(|| Error::Message("OFFER missing busid".into()))?
                .to_string();
            let usbip_port: u16 = port_s
                .parse()
                .map_err(|_| Error::Message(format!("invalid usbip port '{port_s}'")))?;
            if parts.next().is_some() {
                return Err(Error::Message("OFFER has trailing garbage".into()));
            }
            Ok(ClientMsg::Offer { usbip_port, busid })
        }
        "REVOKE" => {
            let busid = parts
                .next()
                .ok_or_else(|| Error::Message("REVOKE missing busid".into()))?
                .to_string();
            Ok(ClientMsg::Revoke { busid })
        }
        "PING" => Ok(ClientMsg::Ping),
        other => Err(Error::Message(format!("unknown control command '{other}'"))),
    }
}

pub fn format_client_msg(msg: &ClientMsg) -> String {
    match msg {
        ClientMsg::Offer { usbip_port, busid } => format!("OFFER {usbip_port} {busid}\n"),
        ClientMsg::Revoke { busid } => format!("REVOKE {busid}\n"),
        ClientMsg::Ping => "PING\n".into(),
    }
}

pub fn parse_server_msg(line: &str) -> Result<ServerMsg> {
    let line = line.trim();
    if line == "OK" {
        return Ok(ServerMsg::Ok);
    }
    if let Some(rest) = line.strip_prefix("ERR ") {
        return Ok(ServerMsg::Err(rest.to_string()));
    }
    if line == "ERR" {
        return Ok(ServerMsg::Err("unknown error".into()));
    }
    Err(Error::Message(format!("bad server reply: {line}")))
}

pub fn format_server_msg(msg: &ServerMsg) -> String {
    match msg {
        ServerMsg::Ok => "OK\n".into(),
        ServerMsg::Err(m) => format!("ERR {m}\n"),
    }
}

fn read_line(stream: &mut impl BufRead) -> Result<String> {
    let mut line = String::new();
    let n = stream
        .read_line(&mut line)
        .map_err(|e| Error::Message(format!("control read failed: {e}")))?;
    if n == 0 {
        return Err(Error::Message("control connection closed".into()));
    }
    Ok(line)
}

fn write_msg(stream: &mut impl Write, text: &str) -> Result<()> {
    stream
        .write_all(text.as_bytes())
        .map_err(|e| Error::Message(format!("control write failed: {e}")))?;
    stream
        .flush()
        .map_err(|e| Error::Message(format!("control flush failed: {e}")))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Server: listen and wait for clients
// ---------------------------------------------------------------------------

pub struct ServeOptions {
    pub bind_addr: String,
    pub control_port: u16,
}

/// Run the server control listener until Ctrl+C.
///
/// Clients connect here and OFFER devices; the server then USB/IP-attaches
/// to the client peer address.
pub fn run_server(opts: ServeOptions) -> Result<()> {
    require_root("run the remote-usb server")?;
    ensure_import_modules()?;

    let bind: SocketAddr = format!("{}:{}", opts.bind_addr, opts.control_port)
        .parse()
        .map_err(|e| Error::Message(format!("invalid bind address: {e}")))?;

    let listener = TcpListener::bind(bind)
        .map_err(|e| Error::Message(format!("failed to bind {bind}: {e}")))?;
    // Don't block forever on accept without checking Ctrl+C.
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::Message(format!("set_nonblocking failed: {e}")))?;

    println!(
        "Server listening on {bind}\n\
         Waiting for clients to attach devices…\n\
         \n\
         On each client:\n\
           sudo remote-usb attach <this-server-ip> <BUSID|VID:PID>\n\
           sudo remote-usb attach <this-server-ip> --auto --match VID:PID\n\
         \n\
         Received devices appear in lsusb on THIS machine.\n\
         Press Ctrl+C to stop."
    );

    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        flag.store(false, Ordering::SeqCst);
    }) {
        tracing::warn!(error = %e, "could not install Ctrl+C handler");
    }

    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let peer_ip = peer.ip();
                println!("client connected from {peer_ip}");
                // Handle one client session (may send multiple OFFER/REVOKE).
                if let Err(e) = handle_client_session(stream, peer_ip.to_string()) {
                    eprintln!("session {peer_ip}: {e}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("accept error: {e}");
                thread::sleep(Duration::from_millis(200));
            }
        }
    }

    println!("Server stopped.");
    Ok(())
}

fn handle_client_session(stream: TcpStream, client_ip: String) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(300)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .ok();

    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|e| Error::Message(format!("clone stream: {e}")))?,
    );
    let mut writer = stream;

    loop {
        let line = match read_line(&mut reader) {
            Ok(l) => l,
            Err(e) => {
                // Idle disconnect is normal.
                tracing::debug!(error = %e, %client_ip, "client session ended");
                break;
            }
        };

        let msg = match parse_client_msg(&line) {
            Ok(m) => m,
            Err(e) => {
                let _ = write_msg(&mut writer, &format_server_msg(&ServerMsg::Err(e.to_string())));
                continue;
            }
        };

        let reply = match msg {
            ClientMsg::Ping => ServerMsg::Ok,
            ClientMsg::Offer { usbip_port, busid } => {
                println!("client {client_ip} offers device {busid} (usbip port {usbip_port})");
                match receive_from_client(&client_ip, usbip_port, &busid) {
                    Ok(()) => {
                        println!(
                            "received {busid} from {client_ip} — check: lsusb / remote-usb ports"
                        );
                        ServerMsg::Ok
                    }
                    Err(e) => {
                        eprintln!("failed to receive {busid} from {client_ip}: {e}");
                        ServerMsg::Err(e.to_string())
                    }
                }
            }
            ClientMsg::Revoke { busid } => {
                println!("client {client_ip} revokes device {busid}");
                match revoke_from_client(&client_ip, &busid) {
                    Ok(true) => {
                        println!("detached {busid} from {client_ip}");
                        ServerMsg::Ok
                    }
                    Ok(false) => ServerMsg::Ok, // already gone
                    Err(e) => ServerMsg::Err(e.to_string()),
                }
            }
        };

        write_msg(&mut writer, &format_server_msg(&reply))?;
        if matches!(reply, ServerMsg::Err(_)) {
            // keep session open for further messages
        }
    }
    Ok(())
}

fn receive_from_client(client_ip: &str, usbip_port: u16, busid: &str) -> Result<()> {
    // Brief wait: client may have just bound the device.
    thread::sleep(Duration::from_millis(200));
    usbip_cmd::attach(client_ip, busid, usbip_port)?;
    Ok(())
}

fn revoke_from_client(client_ip: &str, busid: &str) -> Result<bool> {
    let ports = usbip_cmd::port_list().unwrap_or_default();
    let map = usbip_cmd::attached_remote_busids(&ports);
    if let Some(port_num) = map.get(busid) {
        // Prefer matching client in remote line when possible.
        let remote = ports
            .iter()
            .find(|p| p.port == *port_num)
            .map(|p| p.remote.as_str())
            .unwrap_or("");
        if !remote.is_empty() && !remote.to_ascii_lowercase().contains(&client_ip.to_ascii_lowercase())
        {
            // Still detach by busid if mapped; IP mismatch can happen with hostnames.
            tracing::debug!(%remote, %client_ip, "remote line IP differs; detaching by busid anyway");
        }
        usbip_cmd::detach(*port_num)?;
        return Ok(true);
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Client: connect to server and offer devices
// ---------------------------------------------------------------------------

/// Open a control connection to the server and send one message; wait for OK/ERR.
pub fn send_to_server(server: &str, control_port: u16, msg: &ClientMsg) -> Result<()> {
    let addr = format!("{server}:{control_port}");
    let mut stream = TcpStream::connect(&addr)
        .map_err(|e| Error::Message(format!("cannot connect to server at {addr}: {e}")))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .ok();

    write_msg(&mut stream, &format_client_msg(msg))?;
    let mut reader = BufReader::new(&stream);
    let line = read_line(&mut reader)?;
    match parse_server_msg(&line)? {
        ServerMsg::Ok => Ok(()),
        ServerMsg::Err(e) => Err(Error::Message(format!("server rejected offer: {e}"))),
    }
}

/// Long-lived control session for multiple OFFER/REVOKE (auto mode).
pub struct ControlSession {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
}

impl ControlSession {
    pub fn connect(server: &str, control_port: u16) -> Result<Self> {
        let addr = format!("{server}:{control_port}");
        let stream = TcpStream::connect(&addr)
            .map_err(|e| Error::Message(format!("cannot connect to server at {addr}: {e}")))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .ok();
        stream
            .set_write_timeout(Some(Duration::from_secs(30)))
            .ok();
        let reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|e| Error::Message(format!("clone: {e}")))?,
        );
        Ok(Self { stream, reader })
    }

    pub fn request(&mut self, msg: &ClientMsg) -> Result<()> {
        write_msg(&mut self.stream, &format_client_msg(msg))?;
        let line = read_line(&mut self.reader)?;
        match parse_server_msg(&line)? {
            ServerMsg::Ok => Ok(()),
            ServerMsg::Err(e) => Err(Error::Message(format!("server rejected: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_offer() {
        let m = parse_client_msg("OFFER 3240 1-6\n").unwrap();
        assert_eq!(
            m,
            ClientMsg::Offer {
                usbip_port: 3240,
                busid: "1-6".into()
            }
        );
    }

    #[test]
    fn parse_revoke_ping() {
        assert_eq!(
            parse_client_msg("REVOKE 1-6").unwrap(),
            ClientMsg::Revoke {
                busid: "1-6".into()
            }
        );
        assert_eq!(parse_client_msg("PING").unwrap(), ClientMsg::Ping);
    }

    #[test]
    fn roundtrip_server_msg() {
        assert_eq!(parse_server_msg("OK").unwrap(), ServerMsg::Ok);
        assert_eq!(
            parse_server_msg("ERR boom").unwrap(),
            ServerMsg::Err("boom".into())
        );
    }
}
