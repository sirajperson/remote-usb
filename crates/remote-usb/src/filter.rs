use crate::error::{Error, Result};
use crate::usbip_cmd::{normalize_vid_pid, looks_like_vid_pid, UsbDevice};

/// Filter for which devices to auto-export or auto-attach.
///
/// - If `match_ids` is non-empty, only those VID:PID values are allowed.
/// - `exclude_ids` always removes matches.
/// - USB hubs (device class 0x09) are skipped unless `include_hubs` is set.
#[derive(Debug, Clone, Default)]
pub struct DeviceFilter {
    pub match_ids: Vec<String>,
    pub exclude_ids: Vec<String>,
    pub include_hubs: bool,
}

impl DeviceFilter {
    pub fn from_cli(match_ids: &[String], exclude_ids: &[String], include_hubs: bool) -> Result<Self> {
        let match_ids = normalize_id_list(match_ids)?;
        let exclude_ids = normalize_id_list(exclude_ids)?;
        Ok(Self {
            match_ids,
            exclude_ids,
            include_hubs,
        })
    }

    pub fn allows(&self, dev: &UsbDevice) -> bool {
        let vid_pid = normalize_vid_pid(&dev.vid_pid);

        if self.exclude_ids.iter().any(|id| id == &vid_pid) {
            return false;
        }

        if !self.match_ids.is_empty() && !self.match_ids.iter().any(|id| id == &vid_pid) {
            return false;
        }

        if !self.include_hubs && is_hub(&dev.busid) {
            return false;
        }

        // Skip root-hub style busids just in case they appear.
        if is_root_hub_busid(&dev.busid) {
            return false;
        }

        true
    }

    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.match_ids.is_empty() {
            parts.push("all non-hub devices".to_string());
        } else {
            parts.push(format!("match {}", self.match_ids.join(", ")));
        }
        if !self.exclude_ids.is_empty() {
            parts.push(format!("exclude {}", self.exclude_ids.join(", ")));
        }
        if self.include_hubs {
            parts.push("including hubs".to_string());
        }
        parts.join("; ")
    }
}

fn normalize_id_list(ids: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(ids.len());
    for raw in ids {
        let s = raw.trim();
        if !looks_like_vid_pid(s) {
            return Err(Error::Message(format!(
                "invalid VID:PID filter '{raw}' (expected form abcd:1234)"
            )));
        }
        out.push(normalize_vid_pid(s));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// USB hub device class is 0x09.
fn is_hub(busid: &str) -> bool {
    device_class(busid) == Some(0x09)
}

fn device_class(busid: &str) -> Option<u8> {
    let path = format!("/sys/bus/usb/devices/{busid}/bDeviceClass");
    let s = std::fs::read_to_string(path).ok()?;
    u8::from_str_radix(s.trim(), 16).ok()
}

fn is_root_hub_busid(busid: &str) -> bool {
    // e.g. "usb1", "1-0"
    busid.starts_with("usb") || busid.ends_with("-0")
}

/// Bound to usbip-host means already exported.
pub fn is_exported(busid: &str) -> bool {
    driver_name(busid).as_deref() == Some("usbip-host")
}

pub fn driver_name(busid: &str) -> Option<String> {
    let path = format!("/sys/bus/usb/devices/{busid}/driver");
    std::fs::read_link(path)
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(busid: &str, vid_pid: &str) -> UsbDevice {
        UsbDevice {
            busid: busid.into(),
            vid_pid: vid_pid.into(),
            product: String::new(),
        }
    }

    #[test]
    fn match_and_exclude() {
        let f = DeviceFilter::from_cli(
            &["14cd:1212".into()],
            &["04d9:fc4d".into()],
            false,
        )
        .unwrap();
        assert!(f.allows(&dev("1-6", "14cd:1212")));
        assert!(!f.allows(&dev("1-9", "04d9:fc4d")));
        assert!(!f.allows(&dev("1-3", "18d1:4ee2")));
    }

    #[test]
    fn empty_match_allows_all_non_excluded() {
        let f = DeviceFilter::from_cli(&[], &["04d9:fc4d".into()], false).unwrap();
        assert!(f.allows(&dev("1-6", "14cd:1212")));
        assert!(!f.allows(&dev("1-9", "04d9:fc4d")));
    }

    #[test]
    fn invalid_vid_pid() {
        assert!(DeviceFilter::from_cli(&["not-an-id".into()], &[], false).is_err());
    }
}
