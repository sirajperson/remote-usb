<p align="center">
  <img src="assets/logo.jpg" alt="remote-usb logo" width="160" height="160">
</p>

<h1 align="center">remote-usb</h1>

<p align="center">
  <strong>The server receives USB devices. Clients attach their devices to the server.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Linux-0f172a?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/language-Rust-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/license-MIT-6366f1?style=for-the-badge" alt="MIT">
</p>

---

## Direction (read this once)

| Machine | Role | Command |
|---------|------|---------|
| **Server** | Receives USB devices; they show up in `lsusb` here | `remote-usb serve …` |
| **Client** | Has the physical USB plug; attaches devices **to the server** | `remote-usb attach …` |

```
CLIENT (USB plugged in)                    SERVER (uses the device)
───────────────────────                    ───────────────────────
remote-usb attach <SERVER_IP> 1-6   ──►    remote-usb serve 0.0.0.0 --client <CLIENT_IP>
                                           lsusb   # device appears HERE
```

- **`serve`** is only ever the **server**.
- **`attach`** is only ever the **client** attaching devices to that server.
- The server does **not** export or share its own USB devices.

---

## Manual

### 1. Start the server

On the machine that should **receive** the devices:

```bash
sudo remote-usb serve 0.0.0.0 --client 192.168.1.10 --auto
```

- `0.0.0.0` — server available on all interfaces  
- `--client 192.168.1.10` — IP of the **client** (where the USB stick is plugged in)  
- `--auto` — keep receiving devices as the client attaches them  

Or prepare only:

```bash
sudo remote-usb serve 0.0.0.0
```

### 2. Client attaches its devices to the server

On the machine **with the physical USB**:

```bash
remote-usb list
sudo remote-usb attach 192.168.1.20 1-6
# or by VID:PID:
sudo remote-usb attach 192.168.1.20 14cd:1212
# or auto:
sudo remote-usb attach 192.168.1.20 --auto --match 14cd:1212
```

`192.168.1.20` is the **server** IP (the machine running `serve`).

### 3. Confirm on the server

```bash
lsusb
remote-usb ports
ls -l /dev/disk/by-id/    # mass storage
```

### One-shot import (no --auto on server)

```bash
# Client still: sudo remote-usb attach <SERVER> 1-6
# Server:
sudo remote-usb serve --client 192.168.1.10 list
sudo remote-usb serve --client 192.168.1.10 import 1-6
lsusb
```

### Tear down

```bash
# Server
sudo remote-usb detach 0          # port from `remote-usb ports`

# Client
# Ctrl+C the `attach` process
# or: sudo remote-usb unbind 1-6
```

---

## CLI

### Server

```text
remote-usb serve [0.0.0.0] --client <CLIENT_IP> --auto
remote-usb serve --client <CLIENT_IP> list
remote-usb serve --client <CLIENT_IP> import <BUSID|VID:PID>
remote-usb serve prepare

remote-usb ports
remote-usb detach <VHCI_PORT>
```

### Client

```text
remote-usb list
remote-usb prepare
remote-usb attach <SERVER_IP> <BUSID|VID:PID>
remote-usb attach <SERVER_IP> --auto [--match VID:PID]...
remote-usb unbind <BUSID|VID:PID>
```

```bash
remote-usb --help
remote-usb serve --help
remote-usb attach --help
```

---

## Requirements

- Linux + USB/IP modules (`usbip_core`, `usbip_host`, `vhci_hcd`)
- `usbip`, `usbipd` (e.g. `linux-tools-generic` on Ubuntu)
- Root for `serve`, `attach`, `detach`, `prepare`

## Install

```bash
git clone https://github.com/sirajperson/remote-usb.git
cd remote-usb
cargo build --release -p remote-usb
sudo install -Dm755 target/release/remote-usb /usr/local/bin/remote-usb
```

## Security

Plain TCP (port **3240**), no authentication. Trusted LAN/VPN only.

On the **client**, allow the server:

```bash
sudo ufw allow from <SERVER_IP> to any port 3240 proto tcp
```

## systemd

| File | Role |
|------|------|
| [`systemd/remote-usb-client.service`](systemd/remote-usb-client.service) | Client `attach … --auto` |
| [`systemd/remote-usb-server-attach.service.example`](systemd/remote-usb-server-attach.service.example) | Server `serve … --auto` |

## License

[MIT](LICENSE)
