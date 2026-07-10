use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::error::{Error, Result};

const DEFAULT_PORT: u16 = 3240;

/// A USB device entry from `usbip list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    pub busid: String,
    /// Lowercase `vid:pid`, e.g. `0781:5581`.
    pub vid_pid: String,
    pub product: String,
}

/// An imported (attached) device from `usbip port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedPort {
    pub port: u32,
    pub status: String,
    pub speed: String,
    pub product: String,
    pub remote: String,
}

/// Locate the `usbip` binary on PATH or common linux-tools paths.
pub fn find_usbip() -> Result<PathBuf> {
    find_tool("usbip")
}

/// Locate the `usbipd` binary.
pub fn find_usbipd() -> Result<PathBuf> {
    find_tool("usbipd")
}

fn find_tool(name: &str) -> Result<PathBuf> {
    if let Ok(path) = which(name) {
        return Ok(path);
    }

    // Ubuntu/Debian often ship versioned tools under linux-tools.
    let tools_root = Path::new("/usr/lib/linux-tools");
    if tools_root.is_dir() {
        if let Ok(entries) = std::fs::read_dir(tools_root) {
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path().join(name))
                .filter(|p| p.is_file())
                .collect();
            // Prefer lexicographically last (often newest kernel version dir).
            candidates.sort();
            if let Some(path) = candidates.pop() {
                return Ok(path);
            }
        }
    }

    for dir in ["/usr/sbin", "/sbin", "/usr/bin", "/bin"] {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(Error::ToolNotFound(name.to_string()))
}

fn which(name: &str) -> std::result::Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}

fn run_usbip(args: &[&str]) -> Result<String> {
    let bin = find_usbip()?;
    let mut cmd = Command::new(&bin);
    cmd.args(args);
    tracing::debug!(?bin, ?args, "running usbip");

    let output = cmd.output().map_err(|e| {
        Error::Message(format!("failed to execute {}: {e}", bin.display()))
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let cmd_str = format!("{} {}", bin.display(), args.join(" "));
        return Err(Error::UsbipFailed {
            cmd: cmd_str,
            stderr: if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            },
        });
    }

    if !stderr.is_empty() {
        tracing::debug!(%stderr, "usbip stderr");
    }
    Ok(stdout)
}

fn run_usbip_with_port(port: u16, args: &[&str]) -> Result<String> {
    if port == DEFAULT_PORT {
        return run_usbip(args);
    }
    let port_s = port.to_string();
    let mut full = vec!["--tcp-port", port_s.as_str()];
    full.extend_from_slice(args);
    run_usbip(&full)
}

/// List local USB devices (`usbip list --local`).
pub fn list_local() -> Result<Vec<UsbDevice>> {
    // Prefer human-readable output so product names are available.
    match run_usbip(&["list", "--local"]) {
        Ok(out) => {
            let human = parse_human_local_list(&out)?;
            if !human.is_empty() {
                return Ok(human);
            }
            // Empty human parse: try parsable as a fallback.
            if let Ok(pout) = run_usbip(&["list", "-p", "-l"]) {
                return parse_parsable_list(&pout);
            }
            Ok(human)
        }
        Err(e) => {
            // Fall back to parsable if human listing failed.
            match run_usbip(&["list", "-p", "-l"]) {
                Ok(out) => parse_parsable_list(&out),
                Err(_) => Err(e),
            }
        }
    }
}

/// List devices exported by a remote host.
#[allow(dead_code)]
pub fn list_remote(host: &str, port: u16) -> Result<Vec<UsbDevice>> {
    let remote_arg = format!("--remote={host}");
    match run_usbip_with_port(port, &["list", "-p", remote_arg.as_str()]) {
        Ok(out) if !out.trim().is_empty() && looks_like_parsable(&out) => {
            parse_parsable_list(&out)
        }
        Ok(out) if !out.trim().is_empty() => parse_human_remote_list(&out),
        Ok(_) => Ok(Vec::new()),
        Err(e) => {
            // Retry without -p (some builds reject it for remote).
            if matches!(&e, Error::UsbipFailed { .. }) {
                let out = run_usbip_with_port(port, &["list", remote_arg.as_str()])?;
                parse_human_remote_list(&out)
            } else {
                Err(e)
            }
        }
    }
}

/// Bind (export) a device by busid.
pub fn bind(busid: &str) -> Result<()> {
    let arg = format!("--busid={busid}");
    run_usbip(&["bind", arg.as_str()])?;
    Ok(())
}

/// Unbind a device by busid.
pub fn unbind(busid: &str) -> Result<()> {
    let arg = format!("--busid={busid}");
    run_usbip(&["unbind", arg.as_str()])?;
    Ok(())
}

/// Attach a remote device.
pub fn attach(host: &str, busid: &str, port: u16) -> Result<()> {
    let remote = format!("--remote={host}");
    let bus = format!("--busid={busid}");
    run_usbip_with_port(port, &["attach", remote.as_str(), bus.as_str()])?;
    Ok(())
}

/// Detach an imported device by VHCI port number.
pub fn detach(port_num: u32) -> Result<()> {
    let arg = format!("--port={port_num}");
    run_usbip(&["detach", arg.as_str()])?;
    Ok(())
}

/// List imported devices (`usbip port`).
pub fn port_list() -> Result<Vec<ImportedPort>> {
    let out = run_usbip(&["port"])?;
    parse_port_list(&out)
}

/// Build a `usbipd` command (foreground, no `-D`).
fn usbipd_command(tcp_port: u16, ipv4_only: bool, ipv6_only: bool) -> Result<Command> {
    let bin = find_usbipd()?;
    let mut cmd = Command::new(&bin);
    if tcp_port != DEFAULT_PORT {
        cmd.arg("--tcp-port").arg(tcp_port.to_string());
    }
    if ipv4_only {
        cmd.arg("--ipv4");
    }
    if ipv6_only {
        cmd.arg("--ipv6");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok(cmd)
}

/// Run `usbipd` in the foreground (blocks until exit).
#[allow(dead_code)]
pub fn serve_foreground(tcp_port: u16, ipv4_only: bool, ipv6_only: bool) -> Result<()> {
    let mut cmd = usbipd_command(tcp_port, ipv4_only, ipv6_only)?;
    tracing::info!(tcp_port, "starting usbipd in foreground");

    let status = cmd.status().map_err(|e| {
        Error::Message(format!("failed to start usbipd: {e}"))
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(Error::Message(format!(
            "usbipd exited with status {status}"
        )))
    }
}

/// Spawn `usbipd` as a child process (for auto-export mode).
pub fn spawn_usbipd(tcp_port: u16, ipv4_only: bool, ipv6_only: bool) -> Result<Child> {
    let mut cmd = usbipd_command(tcp_port, ipv4_only, ipv6_only)?;
    tracing::info!(tcp_port, "spawning usbipd");
    cmd.spawn()
        .map_err(|e| Error::Message(format!("failed to spawn usbipd: {e}")))
}

/// Extract remote busid from a `usbip port` remote line, if present.
///
/// Examples:
/// - `3-1 -> usbip://192.168.1.2:3240/5-2` → `5-2`
/// - `usbip://host:3240/1-6` → `1-6`
pub fn remote_busid_from_port_line(remote: &str) -> Option<String> {
    // Prefer path after last '/' in a usbip:// URL.
    if let Some(url_start) = remote.find("usbip://") {
        let url = &remote[url_start..];
        if let Some(slash) = url.rfind('/') {
            let busid = url[slash + 1..].trim();
            if !busid.is_empty() && !busid.contains(' ') {
                return Some(busid.to_string());
            }
        }
    }
    None
}

/// Map remote busid → local VHCI port for ports that are in use.
pub fn attached_remote_busids(ports: &[ImportedPort]) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    for p in ports {
        let in_use = p.status.to_ascii_lowercase().contains("in use");
        if !in_use {
            continue;
        }
        if let Some(busid) = remote_busid_from_port_line(&p.remote) {
            map.insert(busid, p.port);
        }
    }
    map
}

/// Whether a bind failure looks like "already bound / busy".
pub fn is_already_bound_error(err: &Error) -> bool {
    match err {
        Error::UsbipFailed { stderr, .. } => {
            let s = stderr.to_ascii_lowercase();
            s.contains("already") || s.contains("bound") || s.contains("busy")
        }
        _ => false,
    }
}

/// Whether an attach failure looks like "already attached".
pub fn is_already_attached_error(err: &Error) -> bool {
    match err {
        Error::UsbipFailed { stderr, .. } => {
            let s = stderr.to_ascii_lowercase();
            s.contains("already") || s.contains("busy") || s.contains("in use")
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

/// Resolve a user selector (busid or VID:PID) against a device list.
pub fn resolve_selector(devices: &[UsbDevice], selector: &str) -> Result<UsbDevice> {
    let sel = selector.trim();
    if sel.is_empty() {
        return Err(Error::DeviceNotFound {
            selector: selector.to_string(),
        });
    }

    // Exact busid match first.
    if let Some(dev) = devices.iter().find(|d| d.busid.eq_ignore_ascii_case(sel)) {
        return Ok(dev.clone());
    }

    // VID:PID (with optional leading zeros normalization via lowercase compare).
    if looks_like_vid_pid(sel) {
        let want = normalize_vid_pid(sel);
        let matches: Vec<_> = devices
            .iter()
            .filter(|d| normalize_vid_pid(&d.vid_pid) == want)
            .cloned()
            .collect();
        match matches.len() {
            0 => Err(Error::DeviceNotFound {
                selector: selector.to_string(),
            }),
            1 => Ok(matches.into_iter().next().unwrap()),
            n => Err(Error::AmbiguousSelector {
                selector: selector.to_string(),
                count: n,
            }),
        }
    } else {
        Err(Error::DeviceNotFound {
            selector: selector.to_string(),
        })
    }
}

pub fn looks_like_vid_pid(s: &str) -> bool {
    let parts: Vec<_> = s.split(':').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && parts[0].chars().all(|c| c.is_ascii_hexdigit())
        && parts[1].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn normalize_vid_pid(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    let mut parts = s.split(':');
    let vid = parts.next().unwrap_or("");
    let pid = parts.next().unwrap_or("");
    format!("{vid:0>4}:{pid:0>4}", vid = vid, pid = pid)
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

fn looks_like_parsable(out: &str) -> bool {
    out.lines().any(|l| l.contains("busid=") && l.contains("usbid="))
}

/// Parse `usbip list -p -l` style:
/// `busid=1-6#usbid=14cd:1212#`
pub fn parse_parsable_list(out: &str) -> Result<Vec<UsbDevice>> {
    let mut devices = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let busid = extract_field(line, "busid=").ok_or_else(|| {
            Error::Parse(format!("missing busid in parsable line: {line}"))
        })?;
        let vid_pid = extract_field(line, "usbid=").ok_or_else(|| {
            Error::Parse(format!("missing usbid in parsable line: {line}"))
        })?;
        devices.push(UsbDevice {
            busid,
            vid_pid: vid_pid.to_ascii_lowercase(),
            product: String::new(),
        });
    }
    Ok(devices)
}

fn extract_field(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest.find('#').unwrap_or(rest.len());
    let val = rest[..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

/// Parse human-readable local list:
/// ```text
///  - busid 1-6 (14cd:1212)
///    Super Top : microSD card reader (SY-T18) (14cd:1212)
/// ```
pub fn parse_human_local_list(out: &str) -> Result<Vec<UsbDevice>> {
    let mut devices = Vec::new();
    let mut lines = out.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- busid ") {
            let (busid, vid_pid) = parse_busid_line(rest)?;
            let product = lines
                .next()
                .map(|l| clean_product_line(l, &vid_pid))
                .unwrap_or_default();
            devices.push(UsbDevice {
                busid,
                vid_pid,
                product,
            });
        }
    }
    Ok(devices)
}

/// Parse human-readable remote list (similar structure, often prefixed with Exportable):
/// ```text
/// Exportable USB devices
/// ======================
///  - 192.168.1.2
///         1-6: Super Top : microSD card reader (14cd:1212)
///            : /sys/devices/...
///            : (Defined at Interface level) (00/00/00)
///            :  0 - Mass Storage / ...
/// ```
pub fn parse_human_remote_list(out: &str) -> Result<Vec<UsbDevice>> {
    // Try local-style first (some versions use the same format for remote).
    let local_style = parse_human_local_list(out)?;
    if !local_style.is_empty() {
        return Ok(local_style);
    }

    let mut devices = Vec::new();
    // Pattern: "        1-6: Vendor : Product (vid:pid)"
    for line in out.lines() {
        if let Some(cap) = regex_busid_device_line(line) {
            devices.push(cap);
        }
    }
    Ok(devices)
}

fn parse_busid_line(rest: &str) -> Result<(String, String)> {
    // rest: "1-6 (14cd:1212)" or "1-6.1 (abcd:1234)"
    let rest = rest.trim();
    let open = rest
        .rfind('(')
        .ok_or_else(|| Error::Parse(format!("expected (vid:pid) in: {rest}")))?;
    let close = rest
        .rfind(')')
        .ok_or_else(|| Error::Parse(format!("expected closing ) in: {rest}")))?;
    if close <= open {
        return Err(Error::Parse(format!("malformed busid line: {rest}")));
    }
    let busid = rest[..open].trim().to_string();
    let vid_pid = rest[open + 1..close].trim().to_ascii_lowercase();
    if busid.is_empty() || !looks_like_vid_pid(&vid_pid) {
        return Err(Error::Parse(format!("malformed busid line: {rest}")));
    }
    Ok((busid, vid_pid))
}

fn clean_product_line(line: &str, vid_pid: &str) -> String {
    let mut s = line.trim().to_string();
    // Strip trailing "(vid:pid)" if present.
    let suffix = format!("({vid_pid})");
    if let Some(stripped) = s
        .strip_suffix(&suffix)
        .or_else(|| s.strip_suffix(&suffix.to_ascii_uppercase()))
    {
        s = stripped.trim().to_string();
    }
    s
}

/// Lightweight match for remote device lines without regex crate.
fn regex_busid_device_line(line: &str) -> Option<UsbDevice> {
    let trimmed = line.trim();
    // Expect: "<busid>: <product> (<vid:pid>)"
    let colon = trimmed.find(": ")?;
    let busid = trimmed[..colon].trim();
    if busid.is_empty() || !busid.chars().next()?.is_ascii_digit() {
        return None;
    }
    // busid should look like 1-6 or 1-2.3
    if !busid
        .chars()
        .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
    {
        return None;
    }
    let rest = trimmed[colon + 2..].trim();
    let open = rest.rfind('(')?;
    let close = rest.rfind(')')?;
    if close <= open {
        return None;
    }
    let vid_pid = rest[open + 1..close].trim().to_ascii_lowercase();
    if !looks_like_vid_pid(&vid_pid) {
        return None;
    }
    let product = rest[..open].trim().to_string();
    Some(UsbDevice {
        busid: busid.to_string(),
        vid_pid,
        product,
    })
}

/// Parse `usbip port` output:
/// ```text
/// Imported USB devices
/// ====================
/// Port 00: <Port in Use> at High Speed(480Mbps)
///        SanDisk Corp. : Cruzer Glide (0781:5575)
///        3-1 -> usbip://192.168.1.2:3240/5-2
/// ```
pub fn parse_port_list(out: &str) -> Result<Vec<ImportedPort>> {
    let mut ports = Vec::new();
    let mut lines = out.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Port ") {
            let port_num = parse_port_number(rest)?;
            let status = extract_angle_status(rest).unwrap_or_else(|| rest.to_string());
            let speed = extract_speed(rest).unwrap_or_default();

            let product = lines
                .next()
                .map(|l| l.trim().to_string())
                .unwrap_or_default();
            let remote = lines
                .next()
                .map(|l| l.trim().to_string())
                .unwrap_or_default();

            ports.push(ImportedPort {
                port: port_num,
                status,
                speed,
                product,
                remote,
            });
        }
    }
    Ok(ports)
}

fn parse_port_number(rest: &str) -> Result<u32> {
    // "00: <Port in Use> ..."
    let num_str = rest
        .split(':')
        .next()
        .ok_or_else(|| Error::Parse(format!("bad port line: {rest}")))?
        .trim();
    u32::from_str_radix(num_str, 10)
        .or_else(|_| u32::from_str_radix(num_str, 16))
        .map_err(|_| Error::Parse(format!("invalid port number: {num_str}")))
}

fn extract_angle_status(rest: &str) -> Option<String> {
    let start = rest.find('<')? + 1;
    let end = rest.find('>')?;
    if end > start {
        Some(rest[start..end].to_string())
    } else {
        None
    }
}

fn extract_speed(rest: &str) -> Option<String> {
    let at = rest.find(" at ")?;
    Some(rest[at + 4..].trim().to_string())
}

/// Format devices as a simple table for CLI output.
pub fn format_device_table(devices: &[UsbDevice]) -> String {
    if devices.is_empty() {
        return "No devices found.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!(
        "{:<12} {:<11} {}",
        "BUSID", "VID:PID", "PRODUCT"
    ));
    lines.push(format!(
        "{:<12} {:<11} {}",
        "-----", "-------", "-------"
    ));
    for d in devices {
        let product = if d.product.is_empty() {
            "-"
        } else {
            d.product.as_str()
        };
        lines.push(format!(
            "{:<12} {:<11} {}",
            d.busid, d.vid_pid, product
        ));
    }
    lines.join("\n")
}

pub fn format_port_table(ports: &[ImportedPort]) -> String {
    if ports.is_empty() {
        return "No imported devices.".to_string();
    }
    let mut lines = Vec::new();
    lines.push(format!(
        "{:<6} {:<16} {:<20} {}",
        "PORT", "STATUS", "SPEED", "REMOTE / PRODUCT"
    ));
    lines.push(format!(
        "{:<6} {:<16} {:<20} {}",
        "----", "------", "-----", "---------------"
    ));
    for p in ports {
        let detail = if p.remote.is_empty() {
            p.product.clone()
        } else {
            format!("{} | {}", p.remote, p.product)
        };
        lines.push(format!(
            "{:<6} {:<16} {:<20} {}",
            p.port, p.status, p.speed, detail
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const PARSABLE_LOCAL: &str = "\
busid=1-10#usbid=17ef:608c#
busid=1-14#usbid=1532:0537#
busid=1-3#usbid=18d1:4ee2#
busid=1-6#usbid=14cd:1212#
busid=1-9#usbid=04d9:fc4d#
";

    const HUMAN_LOCAL: &str = "\
 - busid 1-10 (17ef:608c)
   Lenovo : unknown product (17ef:608c)

 - busid 1-6 (14cd:1212)
   Super Top : microSD card reader (SY-T18) (14cd:1212)
";

    const HUMAN_REMOTE: &str = "\
Exportable USB devices
======================
 - 192.168.1.10
        1-6: Super Top : microSD card reader (14cd:1212)
           : /sys/devices/pci0000:00/0000:00:14.0/usb1/1-6
           : (Defined at Interface level) (00/00/00)
           :  0 - Mass Storage / SCSI / Bulk-Only (08/06/50)
";

    const PORT_OUT: &str = "\
Imported USB devices
====================
Port 00: <Port in Use> at High Speed(480Mbps)
       SanDisk Corp. : Cruzer Glide (0781:5575)
       3-1 -> usbip://192.168.1.2:3240/5-2

Port 01: <Port Available> at Unknown Speed(0Mbps)

";

    #[test]
    fn parse_parsable() {
        let devs = parse_parsable_list(PARSABLE_LOCAL).unwrap();
        assert_eq!(devs.len(), 5);
        assert_eq!(devs[3].busid, "1-6");
        assert_eq!(devs[3].vid_pid, "14cd:1212");
    }

    #[test]
    fn parse_human_local() {
        let devs = parse_human_local_list(HUMAN_LOCAL).unwrap();
        assert_eq!(devs.len(), 2);
        assert_eq!(
            devs[1],
            UsbDevice {
                busid: "1-6".into(),
                vid_pid: "14cd:1212".into(),
                product: "Super Top : microSD card reader (SY-T18)".into(),
            }
        );
    }

    #[test]
    fn parse_human_remote() {
        let devs = parse_human_remote_list(HUMAN_REMOTE).unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].busid, "1-6");
        assert_eq!(devs[0].vid_pid, "14cd:1212");
        assert!(devs[0].product.contains("Super Top"));
    }

    #[test]
    fn parse_ports() {
        let ports = parse_port_list(PORT_OUT).unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].port, 0);
        assert_eq!(ports[0].status, "Port in Use");
        assert!(ports[0].product.contains("SanDisk"));
        assert!(ports[0].remote.contains("usbip://"));
        assert_eq!(ports[1].port, 1);
        assert_eq!(ports[1].status, "Port Available");
    }

    #[test]
    fn resolve_by_busid_and_vidpid() {
        let devs = parse_parsable_list(PARSABLE_LOCAL).unwrap();
        let d = resolve_selector(&devs, "1-6").unwrap();
        assert_eq!(d.vid_pid, "14cd:1212");

        let d = resolve_selector(&devs, "14CD:1212").unwrap();
        assert_eq!(d.busid, "1-6");

        assert!(matches!(
            resolve_selector(&devs, "ffff:ffff"),
            Err(Error::DeviceNotFound { .. })
        ));
    }

    #[test]
    fn normalize_vid_pid_padding() {
        assert_eq!(normalize_vid_pid("14cd:1212"), "14cd:1212");
        assert_eq!(normalize_vid_pid("4d9:fc4d"), "04d9:fc4d");
    }

    #[test]
    fn remote_busid_from_port() {
        assert_eq!(
            remote_busid_from_port_line("3-1 -> usbip://192.168.1.2:3240/5-2").as_deref(),
            Some("5-2")
        );
        assert_eq!(
            remote_busid_from_port_line("usbip://host:3240/1-6").as_deref(),
            Some("1-6")
        );
        assert_eq!(remote_busid_from_port_line("nothing here"), None);
    }

    #[test]
    fn attached_map() {
        let ports = parse_port_list(PORT_OUT).unwrap();
        let map = attached_remote_busids(&ports);
        assert_eq!(map.get("5-2"), Some(&0));
        assert!(!map.contains_key("1-6"));
    }
}
