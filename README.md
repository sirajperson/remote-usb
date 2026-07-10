<p align="center">
  <img src="assets/logo.jpg" alt="remote-usb logo" width="160" height="160">
</p>

<h1 align="center">remote-usb</h1>

<p align="center">
  <strong>Clients share USB devices with a server over the network</strong><br>
  Plug USB into a <em>client</em> — use it on the <em>server</em> as if it were local.
</p>

<p align="center">
  <a href="#quick-start"><img src="https://img.shields.io/badge/quick%20start-get%20going-22d3ee?style=for-the-badge" alt="Quick start"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT-6366f1?style=for-the-badge" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/platform-Linux-0f172a?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/language-Rust-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
</p>

---

## Mental model

The **server imports** USB devices from **clients**. Clients **export** devices to the server. Once imported, the server sees them like normal local USB hardware (`lsusb`, `/dev/…`).

| Role | Where the stick is | What it does |
|------|--------------------|--------------|
| **Server** | Uses remote devices | Imports / mounts devices from clients |
| **Client** | Physical USB plugged in | Exports devices to the server |

```
┌─────────────────────────┐         network          ┌─────────────────────────┐
│  CLIENT (export)        │  ─────────────────────►  │  SERVER (import)        │
│  physical USB           │   client exports devices │  imports & uses them    │
│  remote-usb share/bind  │                          │  visible in lsusb       │
└─────────────────────────┘                          └─────────────────────────┘
```

Default commands (`list`, `bind`, `share`) run on the **client**.  
`remote-usb server …` runs on the **server**.

> ⚠️ **Security:** plain TCP, no authentication. Trusted LAN/VPN only. Do not expose port `3240` to the internet.

---

## Quick start

### Direct attachment (automatic)

**1. Server** (imports devices from the client) — leave running:

```bash
sudo remote-usb server prepare
sudo remote-usb server --client 192.168.1.10 --auto --match 14cd:1212
```

**2. Client** (exports its USB to the server) — leave running:

```bash
sudo remote-usb share --auto --match 14cd:1212
# same as:
sudo remote-usb share 0.0.0.0 --auto --match 14cd:1212
```

Plug/unplug on the client; the server imports/detaches within a few seconds.

**3. On the server**, confirm the device is local:

```bash
lsusb
remote-usb ports
ls -l /dev/disk/by-id/    # mass storage
```

> Prefer `--match VID:PID`. Without it, client `--auto` may export **all** non-hub devices (including keyboard/mouse).

Find IDs on the client with `remote-usb list`.

### Manual

The server loads first, then the client exports, then the server imports a device.

**1. Server — prepare to import**

```bash
sudo remote-usb server prepare
```

**2. Client — export devices to the server**

```bash
sudo remote-usb prepare
remote-usb list                         # note busid or VID:PID
sudo remote-usb share 0.0.0.0           # leave running
sudo remote-usb bind 1-6                # other terminal: export this device
# or: sudo remote-usb bind 14cd:1212
```

**3. Server — import a device from the client**

```bash
# CLIENT_IP = address of the machine running `share`
remote-usb server --client 192.168.1.10 list
sudo remote-usb server --client 192.168.1.10 bind 1-6
```

**4. Server — device is now local**

```bash
lsusb                    # device appears here on the server
remote-usb ports         # remote-usb attachment status
ls -l /dev/disk/by-id/   # mass-storage nodes, if applicable
```

Your desktop or `udisks` may auto-mount storage after import.

**Tear down**

```bash
# Server: stop using the device
sudo remote-usb detach 0               # VHCI port from `remote-usb ports`

# Client: stop exporting
sudo remote-usb unbind 1-6
# Ctrl+C on `share` / `server --auto`
```

---

## CLI

### Client (default — USB plugged in here)

```text
remote-usb list
remote-usb bind <BUSID|VID:PID>
remote-usb unbind <BUSID|VID:PID>
remote-usb prepare

remote-usb share [0.0.0.0] [--port 3240]
    [--auto] [--match VID:PID]... [--exclude VID:PID]...
    [--interval SECS] [--no-unbind-on-exit]
```

### Server (imports devices from clients)

```text
remote-usb server prepare
remote-usb server --client <CLIENT_IP> list                 # devices client is exporting
remote-usb server --client <CLIENT_IP> bind <BUSID|VID:PID> # import one device (alias: attach)
remote-usb server --client <CLIENT_IP> --auto [--match …]   # keep importing as client exports

remote-usb ports              # devices already imported on this server
remote-usb detach <VHCI_PORT>
```

```bash
remote-usb --help
remote-usb share --help
remote-usb server --help
```

| Variable / flag | Meaning |
|-----------------|---------|
| `REMOTE_USB_PORT` | Default TCP port (`3240`) |
| `-v` / `-vv` / `RUST_LOG` | Logging |

---

## Requirements

- **Linux** with USB/IP modules (`usbip_core`, `usbip_host`, `vhci_hcd`)
- `usbip` and `usbipd`
  - Debian/Ubuntu: `sudo apt install linux-tools-generic`
  - Fedora: `sudo dnf install usbip`
  - Arch: `sudo pacman -S usbip`
- **Root** for `share`, `bind`, `server`, `detach`, `prepare`
- [Rust](https://rustup.rs/) to build from source

---

## Install

```bash
git clone https://github.com/sirajperson/remote-usb.git
cd remote-usb
cargo build --release -p remote-usb
sudo install -Dm755 target/release/remote-usb /usr/local/bin/remote-usb
```

---

## How it works

1. **Server** loads import support (`vhci_hcd`) and imports devices from a client  
2. **Client** exports selected USB devices (`share` + `bind`)  
3. Once imported, the **server** sees them as normal USB devices (`lsusb`, `/dev/…`)  

Under the hood this wraps kernel [USB/IP](https://wiki.archlinux.org/title/USB/IP); the CLI orchestrates `usbip` / `usbipd`.

---

## systemd

| File | Role |
|------|------|
| [`systemd/remote-usb-client.service`](systemd/remote-usb-client.service) | Client `share --auto` |
| [`systemd/remote-usb-server-attach.service.example`](systemd/remote-usb-server-attach.service.example) | Server `--client … --auto` |

---

## Firewall

On each **client**, allow the server to reach TCP 3240:

```bash
sudo ufw allow from 192.168.1.20 to any port 3240 proto tcp
```

---

## Limitations (v1)

- Linux only  
- No TLS or authentication  
- Busids can change across reboot — prefer unique `VID:PID`  
- Auto mode polls on an interval (not udev-native)  
- One `--client` per `server` process for now (run multiple processes for multiple clients)  

---

## License

[MIT](LICENSE)
