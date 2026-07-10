use crate::error::{Error, Result};

/// Return true if the process has effective UID 0.
pub fn is_root() -> bool {
    effective_uid().is_some_and(|uid| uid == 0)
}

/// Fail with a clear message if not root.
pub fn require_root(action: &str) -> Result<()> {
    if is_root() {
        Ok(())
    } else {
        Err(Error::PrivilegeRequired(action.to_string()))
    }
}

/// Read effective UID from `/proc/self/status` (Linux).
fn effective_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    // Format: "Uid:\treal\teffective\tsaved\tfs"
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let mut fields = rest.split_whitespace();
            let _real = fields.next()?;
            let effective = fields.next()?;
            return effective.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_uid_is_readable() {
        assert!(effective_uid().is_some());
    }
}
