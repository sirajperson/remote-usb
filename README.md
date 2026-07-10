<p align="center">
  <img src="assets/logo.jpg" alt="remote-usb logo" width="160" height="160">
</p>

<h1 align="center">remote-usb</h1>

<p align="center">
  <strong>The server waits. Clients attach their USB devices to it.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Linux-0f172a?style=for-the-badge&logo=linux&logoColor=white" alt="Linux">
  <img src="https://img.shields.io/badge/language-Rust-dea584?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/license-MIT-6366f1?style=for-the-badge" alt="MIT">
</p>

---

## Architecture (normal client/server)

```
                 sudo remote-usb serve 0.0.0.0
                 ┌──────────────────────────┐
                 │         SERVER           │
                 │  waits for clients       │
                 │  receives devices        │
                 │  → lsusb shows them      │
                 └───────────▲──────────────┘
                             │
              clients connect and attach
                             │
         ┌───────────────────┴───────────────────┐
         │                                       │
┌────────┴────────┐                     ┌────────┴────────┐
│ CLIENT A        │                     │ CLIENT B        │
│ USB plugged in  │                     │ USB plugged in  │
│ attach SERVER … │                     │ attach SERVER … │
└─────────────────┘                     └─────────────────┘
```

| Role | Runs | Does |
|------|------|------|
| **Server** | `remote-usb serve 0.0.0.0` | Sits and waits. Does **not** need client IPs. |
| **Client** | `remote-usb attach <SERVER_IP> …` | Connects to the server and attaches local USB. |

The server never “calls out” to a preconfigured client. **Clients initiate.**

---

## Manual

### 1. Start the server (waits)

```bash
sudo remote-usb serve 0.0.0.0
```

That’s it. No `--client`. Leave it running.

### 2. Client attaches a device to the server

On the machine **with the USB stick**:

```bash
remote-usb list
sudo remote-usb attach 192.168.1.20 1-6
# or:
sudo remote-usb attach 192.168.1.20 14cd:1212
# or auto:
sudo remote-usb attach 192.168.1.20 --auto --match 14cd:1212
```

`192.168.1.20` = the machine running `serve`.

### 3. On the server — device is local

```bash
lsusb
remote-usb ports
```

### Tear down

- Client: Ctrl+C on `attach` (revokes the device from the server)
- Server: Ctrl+C on `serve`, or `sudo remote-usb detach 0`

---

## CLI

### Server

```text
remote-usb serve [0.0.0.0] [--control-port 3250]
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

---

## Ports

| Port | Purpose |
|------|---------|
| **3250** | Control (clients connect to `serve`) |
| **3240** | USB/IP data (used after a client offers a device) |

Firewall: allow **3250** (and **3240** from the server back to the client) on a trusted LAN.

```bash
# On server: clients connect in
sudo ufw allow 3250/tcp

# On client: server pulls USB/IP data after the client offers a device
sudo ufw allow from <SERVER_IP> to any port 3240 proto tcp
```

---

## Install

```bash
git clone https://github.com/sirajperson/remote-usb.git
cd remote-usb
cargo build --release -p remote-usb
sudo install -Dm755 target/release/remote-usb /usr/local/bin/remote-usb
```

Requirements: Linux, `usbip`/`usbipd`, kernel modules `usbip_core`, `usbip_host`, `vhci_hcd`.

---

## How it works

1. **Server** listens on the control port and waits.  
2. **Client** binds a local USB device and **connects to the server** with an offer.  
3. **Server** accepts the offer and attaches the device so it appears in `lsusb`.  

You never pass `--client` to the server. Client addresses come from the TCP connection.

---

## Security

Plain TCP, no authentication. Trusted networks only.

## License

[MIT](LICENSE)
