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

| Role | Where USB is | What it does |
|------|----------------|--------------|
| **Client** | Plugged in here | Shares devices with the server |
| **Server** | Uses remote devices | Attaches devices that clients share |

```
┌─────────────────────────┐         network          ┌─────────────────────────┐
│  CLIENT                 │  ─────────────────────►  │  SERVER                 │
│  physical USB           │   client shares devices  │  uses those devices     │
│  remote-usb share       │                          │  remote-usb server …    │
│  remote-usb bind        │                          │  remote-usb ports       │
└─────────────────────────┘                          └─────────────────────────┘
```

Default commands (`list`, `bind`, `share`) are for the **client**.  
`remote-usb server …` is for the **server**.

> ⚠️ **Security:** plain TCP, no authentication. Trusted LAN/VPN only. Do not expose port `3240` to the internet.

---

## Quick start

### Direct attachment (automatic)

**Client** (USB plugged in) — leave running:

```bash
sudo remote-usb share --auto --match 14cd:1212
# same as:
sudo remote-usb share 0.0.0.0 --auto --match 14cd:1212
```

**Server** (uses the client's devices) — leave running:

```bash
sudo remote-usb server --client 192.168.1.10 --auto --match 14cd:1212
# optional bind note:
sudo remote-usb server 0.0.0.0 --client 192.168.1.10 --auto --match 14cd:1212
```

Plug/unplug on the client; the server attaches/detaches within a few seconds.

> Prefer `--match VID:PID`. Without it, client `--auto` may share **all** non-hub devices (including keyboard/mouse).

Find IDs on the client:

```bash
remote-usb list
```

### Manual

**Client** (USB plugged in here):

```bash
sudo remote-usb prepare
remote-usb list
sudo remote-usb share 0.0.0.0          # leave running
sudo remote-usb bind 1-6               # other terminal
# or: sudo remote-usb bind 14cd:1212
```

**Server** (use the client's device):

```bash
sudo remote-usb server prepare
remote-usb server --client 192.168.1.10 list
sudo remote-usb server --client 192.168.1.10 bind 1-6
remote-usb ports
```

**Tear down:**

```bash
sudo remote-usb detach 0               # on server (port from `ports`)
sudo remote-usb unbind 1-6             # on client
# Ctrl+C share / server --auto
```

After the server attaches a device:

```bash
lsusb
ls -l /dev/disk/by-id/
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

### Server (uses devices from clients)

```text
remote-usb server prepare
remote-usb server --client <CLIENT_IP> list
remote-usb server --client <CLIENT_IP> bind <BUSID|VID:PID>   # alias: attach
remote-usb server --client <CLIENT_IP> --auto [--match VID:PID]...
remote-usb server 0.0.0.0 --client <CLIENT_IP> --auto

remote-usb ports
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

Product language: **clients share, server uses.**

Under the hood this wraps kernel [USB/IP](https://wiki.archlinux.org/title/USB/IP):

1. Client runs an export listener (`share` → `usbipd`) and binds devices  
2. Server connects to the client and attaches devices (`vhci_hcd`)  
3. Optional auto loops keep both sides in sync  

You do not reimplement the wire protocol; the CLI orchestrates system tools.

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
