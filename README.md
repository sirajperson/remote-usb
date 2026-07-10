<p align="center">
  <img src="assets/logo.jpg" alt="remote-usb logo" width="160" height="160">
</p>

<h1 align="center">remote-usb</h1>

<p align="center">
  <strong>Share USB devices over your network on Linux</strong><br>
  Plug a device into a <em>client</em> machine — use it on a <em>server</em> as if it were local.
</p>

<p align="center">
  <a href="#quick-start"><img src="https://img.shields.io/badge/quick%20start-get%20going-22d3ee?style=for-the-badge" alt="Quick start"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT-6366f1?style=for-the-badge" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/platform-Linux-0f172a?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/language-Rust-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
</p>

---

## Why remote-usb?

Need a USB key, serial adapter, or flash drive on a headless box or lab server — but the hardware is on your desk?

**remote-usb** wraps the Linux kernel [USB/IP](https://wiki.archlinux.org/title/USB/IP) stack in a friendly CLI:

- **Direct attachment** — auto-export on the client, auto-attach on the server
- **Manual control** — bind / attach individual devices by busid or `VID:PID`
- **Full USB classes** — storage, HID, serial, and more (whatever the kernel supports)
- **Trusted-network simple** — plain TCP, no auth (v1); perfect for LAN / VPN labs

> ⚠️ **Security:** traffic is unencrypted and unauthenticated. Use only on trusted networks. Do **not** expose TCP port `3240` to the internet.

---

## Architecture

```
┌─────────────────────────┐         TCP :3240          ┌─────────────────────────┐
│  CLIENT (export)        │  ───────────────────────►  │  SERVER (import)        │
│  physical USB device    │      USB/IP protocol       │  device appears in lsusb│
│  usbip_host + usbipd    │                            │  vhci_hcd               │
└─────────────────────────┘                            └─────────────────────────┘
```

| Our term   | Physical USB? | Kernel role | Modules                    |
|------------|---------------|-------------|----------------------------|
| **client** | Yes           | Export      | `usbip_core`, `usbip_host` |
| **server** | No (virtual)  | Import      | `usbip_core`, `vhci_hcd`   |

> Our naming matches *“the server mounts devices from the client.”* Classic USB/IP docs swap “server” and “client.”

---

## Features

| Feature | Description |
|---------|-------------|
| Direct attachment | `client serve --auto` + `server follow` for plug-and-play |
| VID:PID filters | `--match` / `--exclude` so keyboards stay local |
| Manual bind/attach | Full control when you need it |
| Human-friendly CLI | Rich `--help` with examples |
| systemd samples | Units under [`systemd/`](systemd/) |

---

## Requirements

- **Linux** with USB/IP modules (`usbip_core`, `usbip_host`, `vhci_hcd`)
- Userspace tools: `usbip`, `usbipd`
  - Debian/Ubuntu: `sudo apt install linux-tools-generic`  
    (or `linux-tools-$(uname -r)`)
  - Fedora: `sudo dnf install usbip`
  - Arch: `sudo pacman -S usbip`
- **Root** for prepare / serve / bind / attach / follow
- [Rust](https://rustup.rs/) toolchain to build from source

---

## Install

```bash
git clone https://github.com/<you>/remote_usb.git
cd remote_usb
cargo build --release -p remote-usb

# optional: install the binary
sudo install -Dm755 target/release/remote-usb /usr/local/bin/remote-usb
```

Binary path after build: `target/release/remote-usb`

---

## Quick start

### Direct attachment (recommended)

**Client** (USB plugged in here) — leave running:

```bash
sudo remote-usb client serve --auto --match 14cd:1212
```

**Server** (where you use the device) — leave running:

```bash
sudo remote-usb server follow 192.168.1.10 --match 14cd:1212
```

Plug or unplug on the client; within a few seconds the server attaches or detaches.

> Prefer `--match VID:PID`. Without it, `--auto` may export **all** non-hub devices (including keyboard/mouse).

Find IDs with:

```bash
remote-usb client list
```

### Manual mode

<details>
<summary>Click to expand step-by-step manual export / attach</summary>

**Client**

```bash
sudo remote-usb client prepare
remote-usb client list
sudo remote-usb client serve          # leave running
sudo remote-usb client bind 14cd:1212 # other terminal
```

**Server**

```bash
sudo remote-usb server prepare
remote-usb server list 192.168.1.10
sudo remote-usb server attach 192.168.1.10 14cd:1212
remote-usb server ports
```

**Tear down**

```bash
sudo remote-usb server detach 0       # VHCI port from `server ports`
sudo remote-usb client unbind 14cd:1212
# Ctrl+C serve / follow
```

</details>

After attach, mass-storage devices appear under `/dev/disk/by-id/` (desktop/udisks may mount them).

```bash
lsusb
ls -l /dev/disk/by-id/
```

---

## CLI overview

```text
remote-usb client prepare
remote-usb client list
remote-usb client bind <BUSID|VID:PID>
remote-usb client unbind <BUSID|VID:PID>
remote-usb client serve [--port 3240] [--auto]
    [--match VID:PID]... [--exclude VID:PID]...
    [--interval SECS] [--no-unbind-on-exit]

remote-usb server prepare
remote-usb server list <HOST>
remote-usb server attach <HOST> <BUSID|VID:PID>
remote-usb server detach <PORT>
remote-usb server ports
remote-usb server follow <HOST>
    [--match VID:PID]... [--exclude VID:PID]...
    [--interval SECS] [--no-detach-missing]
```

Built-in help (with examples):

```bash
remote-usb --help
remote-usb client serve --help
remote-usb server follow --help
```

| Variable / flag | Meaning |
|-----------------|---------|
| `REMOTE_USB_PORT` | Default TCP port (`3240`) |
| `-v` / `-vv` / `RUST_LOG` | Logging |

---

## systemd

Sample units live in [`systemd/`](systemd/):

| File | Role |
|------|------|
| `remote-usb-client.service` | Long-running client export (`serve --auto`) |
| `remote-usb-server-attach.service.example` | Server `follow` example |

Copy, set the binary path and filters, then enable as needed.

---

## Firewall

USB/IP uses **TCP 3240** by default. On the client, allow only your server:

```bash
sudo ufw allow from 192.168.1.20 to any port 3240 proto tcp
```

---

## How it works

`remote-usb` does **not** reimplement the USB/IP wire protocol. It:

1. Loads the right kernel modules  
2. Wraps system `usbip` / `usbipd`  
3. Parses device lists and accepts **busid** or **VID:PID** selectors  
4. Optionally loops for auto-export / auto-attach  

Device semantics come from the kernel (latency and USB/IP limits still apply).

---

## Limitations (v1)

- Linux only  
- No TLS or authentication  
- Busids can change across reboot — prefer unique `VID:PID`  
- Hotplug is polled (`--interval`), not udev-native  

---

## Project layout

```text
remote_usb/
├── assets/logo.jpg          # project logo
├── crates/remote-usb/       # CLI crate
├── systemd/                 # example unit files
├── Cargo.toml               # workspace
└── README.md
```

---

## Contributing

Issues and PRs welcome. For local checks:

```bash
cargo test -p remote-usb
cargo build --release -p remote-usb
```

---

## License

[MIT](LICENSE) — free to use, modify, and distribute.
