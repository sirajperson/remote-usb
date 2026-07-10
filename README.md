<p align="center">
  <img src="assets/logo.jpg" alt="remote-usb logo" width="160" height="160">
</p>

<h1 align="center">remote-usb</h1>

<p align="center">
  <strong>The server imports USB devices from clients</strong><br>
  Plug USB into a <em>client</em> — it appears on the <em>server</em> (e.g. in <code>lsusb</code>).
</p>

<p align="center">
  <a href="#quick-start"><img src="https://img.shields.io/badge/quick%20start-get%20going-22d3ee?style=for-the-badge" alt="Quick start"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT-6366f1?style=for-the-badge" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/platform-Linux-0f172a?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/language-Rust-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
</p>

---

## Who does what

| Role | Physical USB? | Role in one sentence |
|------|---------------|----------------------|
| **Server** | No | **Imports** devices from clients and uses them (`lsusb` on the server) |
| **Client** | Yes | **Exports** its plugged-in devices so the server can import them |

```
┌──────────────────────────┐                      ┌──────────────────────────┐
│  CLIENT                  │                      │  SERVER                  │
│  USB stick plugged in    │  ──── exports ────►  │  imports the device      │
│                          │                      │  then: lsusb shows it    │
│  remote-usb share        │                      │  remote-usb server …     │
│  remote-usb bind         │                      │  remote-usb ports        │
└──────────────────────────┘                      └──────────────────────────┘
```

**The server does not export devices.** Clients export; the server imports.

> ⚠️ **Security:** plain TCP, no authentication. Trusted LAN/VPN only. Do not expose port `3240` to the internet.

---

## Quick start

### Direct attachment (automatic)

**1. On the SERVER** (imports from the client) — leave running:

```bash
sudo remote-usb server prepare
sudo remote-usb server --client 192.168.1.10 --auto --match 14cd:1212
```

`192.168.1.10` is the **client** (machine with the USB stick).

**2. On the CLIENT** (exports its USB) — leave running:

```bash
sudo remote-usb share --auto --match 14cd:1212
```

**3. On the SERVER** — confirm the device is local:

```bash
lsusb
remote-usb ports
ls -l /dev/disk/by-id/    # if mass storage
```

> Prefer `--match VID:PID` on the client. Without it, `--auto` may export **all** non-hub devices (including keyboard/mouse).

---

### Manual

Order: **server loads → client exports → server imports → check with lsusb**.

#### 1. Server — prepare to import

```bash
sudo remote-usb server prepare
```

#### 2. Client — export a device to the server

```bash
sudo remote-usb prepare
remote-usb list                         # note busid or VID:PID
sudo remote-usb share 0.0.0.0           # leave running (export listener)
sudo remote-usb bind 1-6                # other terminal: export this device
# or: sudo remote-usb bind 14cd:1212
```

#### 3. Server — import that device from the client

```bash
# --client is the CLIENT machine's IP (where `share` is running)
remote-usb server --client 192.168.1.10 list
sudo remote-usb server --client 192.168.1.10 attach 1-6
# `bind` is an alias of `attach` on the server (import), not export
```

#### 4. Server — device is now local

```bash
lsusb                    # device appears on the SERVER
remote-usb ports         # import status
ls -l /dev/disk/by-id/   # mass storage, if any
```

#### Tear down

```bash
# Server: stop importing
sudo remote-usb detach 0               # VHCI port from `remote-usb ports`

# Client: stop exporting
sudo remote-usb unbind 1-6
# Ctrl+C on client `share` and server `--auto`
```

---

## CLI reference

### Client commands (USB plugged in here — export)

```text
remote-usb list                              # local devices on the client
remote-usb bind <BUSID|VID:PID>              # export one device
remote-usb unbind <BUSID|VID:PID>            # stop exporting
remote-usb prepare                           # load client modules
remote-usb share [0.0.0.0] [--auto] …        # export listener
```

### Server commands (import from a client)

```text
remote-usb server prepare
remote-usb server --client <CLIENT_IP> list
remote-usb server --client <CLIENT_IP> attach <BUSID|VID:PID>   # import (alias: bind)
remote-usb server --client <CLIENT_IP> --auto [--match …]

remote-usb ports               # already imported on this server
remote-usb detach <VHCI_PORT>  # stop importing one device
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

1. **Server** loads import support and imports devices from a client  
2. **Client** exports selected USB devices (`share` + `bind`)  
3. On the **server**, imported devices look like normal USB (`lsusb`, `/dev/…`)  

The server never exports its own devices in this design; it only imports from clients.

Under the hood: kernel [USB/IP](https://wiki.archlinux.org/title/USB/IP) via `usbip` / `usbipd`.

---

## systemd

| File | Role |
|------|------|
| [`systemd/remote-usb-client.service`](systemd/remote-usb-client.service) | Client export (`share --auto`) |
| [`systemd/remote-usb-server-attach.service.example`](systemd/remote-usb-server-attach.service.example) | Server import (`--client … --auto`) |

---

## Firewall

On each **client**, allow the **server** to connect to TCP 3240:

```bash
sudo ufw allow from 192.168.1.20 to any port 3240 proto tcp
```

(`192.168.1.20` = server IP.)

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
