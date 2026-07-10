<p align="center">
  <img src="assets/logo.jpg" alt="remote-usb logo" width="160" height="160">
</p>

<h1 align="center">remote-usb</h1>

<p align="center">
  <strong>Share USB devices over your network on Linux</strong><br>
  Plug a device into one machine — use it on another as if it were local.
</p>

<p align="center">
  <a href="#quick-start"><img src="https://img.shields.io/badge/quick%20start-get%20going-22d3ee?style=for-the-badge" alt="Quick start"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT-6366f1?style=for-the-badge" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/platform-Linux-0f172a?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/language-Rust-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
</p>

---

## Mental model

| You want to… | Command style |
|--------------|----------------|
| Share USB devices **plugged into this machine** | Default commands + `server` |
| **Use** USB devices from another machine | `host <ip> …` |

```
┌──────────────────────────┐       TCP :3240        ┌──────────────────────────┐
│  THIS MACHINE (export)   │  ───────────────────►  │  OTHER MACHINE (import)  │
│  remote-usb server       │                        │  remote-usb host <ip> …  │
│  remote-usb bind 1-6     │                        │  remote-usb host <ip> bind│
└──────────────────────────┘                        └──────────────────────────┘
```

> ⚠️ **Security:** plain TCP, no authentication. Trusted LAN/VPN only. Do not expose port `3240` to the internet.

---

## Quick start

### Direct attachment (automatic)

**Machine with the USB device** — leave running:

```bash
sudo remote-usb server --auto --match 14cd:1212
# same as:
sudo remote-usb server 0.0.0.0 --auto --match 14cd:1212
```

**Machine that should use it** — leave running:

```bash
sudo remote-usb host 192.168.1.10 --auto --match 14cd:1212
```

Plug/unplug on the first machine; the second attaches/detaches within a few seconds.

> Prefer `--match VID:PID`. Without it, `--auto` may share **all** non-hub devices (including keyboard/mouse).

Discover IDs:

```bash
remote-usb list
```

### Manual

**Export side** (USB plugged in here):

```bash
sudo remote-usb prepare
remote-usb list
sudo remote-usb server 0.0.0.0          # leave running
sudo remote-usb bind 1-6                # other terminal
# or: sudo remote-usb bind 14cd:1212
```

**Import side** (use the device):

```bash
sudo remote-usb host 192.168.1.10 prepare
remote-usb host 192.168.1.10 list
sudo remote-usb host 192.168.1.10 bind 1-6
remote-usb ports
```

**Tear down:**

```bash
sudo remote-usb detach 0                # VHCI port from `ports`
sudo remote-usb unbind 1-6              # on the export machine
# Ctrl+C server / host --auto
```

After attach, check:

```bash
lsusb
ls -l /dev/disk/by-id/
```

---

## CLI

### Export side (default)

```text
remote-usb list
remote-usb bind <BUSID|VID:PID>
remote-usb unbind <BUSID|VID:PID>
remote-usb prepare

remote-usb server [0.0.0.0] [--port 3240]
    [--auto] [--match VID:PID]... [--exclude VID:PID]...
    [--interval SECS] [--no-unbind-on-exit]
```

### Import side

```text
remote-usb host <IP> prepare
remote-usb host <IP> list
remote-usb host <IP> bind <BUSID|VID:PID>    # alias: attach
remote-usb host <IP> --auto [--match VID:PID]...

remote-usb ports
remote-usb detach <VHCI_PORT>
```

```bash
remote-usb --help
remote-usb server --help
remote-usb host --help
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
- **Root** for `server`, `bind`, `host …`, `detach`, `prepare`
- [Rust](https://rustup.rs/) to build from source

---

## Install

```bash
git clone https://github.com/<you>/remote_usb.git
cd remote_usb
cargo build --release -p remote-usb
sudo install -Dm755 target/release/remote-usb /usr/local/bin/remote-usb
```

---

## Features

| Feature | Description |
|---------|-------------|
| Simple CLI | Export is default; `host <ip>` for import |
| Direct attachment | `server --auto` + `host <ip> --auto` |
| Filters | `--match` / `--exclude` by VID:PID |
| Manual control | `bind` / `host … bind` when you need it |
| systemd samples | See [`systemd/`](systemd/) |

---

## systemd

| File | Role |
|------|------|
| [`systemd/remote-usb-client.service`](systemd/remote-usb-client.service) | Export daemon (`server --auto`) |
| [`systemd/remote-usb-server-attach.service.example`](systemd/remote-usb-server-attach.service.example) | Import (`host <ip> --auto`) |

---

## Firewall

On the export machine, allow only your peer:

```bash
sudo ufw allow from 192.168.1.20 to any port 3240 proto tcp
```

---

## How it works

Wraps kernel USB/IP (`usbip` / `usbipd`): loads modules, binds/attaches devices, optional auto loops. Does not reimplement the wire protocol.

---

## Limitations (v1)

- Linux only  
- No TLS or authentication  
- Busids can change across reboot — prefer unique `VID:PID`  
- Polling interval for auto mode (not udev-native)  
- `server <addr>` accepts an address for clarity; `usbipd` listens on all interfaces — restrict with firewall  

---

## License

[MIT](LICENSE)
