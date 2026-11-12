//! Linux elevation shim.
//!
//! The Windows bootstrap re-executes the manager through the UAC `runas` verb.
//! On Linux there is nothing to elevate: the manager edits a Wine prefix, which
//! is an ordinary user-owned directory. This shim therefore reports "already
//! allowed" and the caller continues in-process.
//!
//! The `Windows` implementation returns `Ok(false)` to mean "a privileged copy
//! was launched, exit now"; returning `Ok(true)` here is the honest inverse —
//! there is no second process, so the current one must keep going.

use anyhow::Result;
use std::ffi::OsString;

/// Always `Ok(true)`: no elevation is required or possible on Linux.
pub fn ensure_admin(_args: &[OsString]) -> Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_reports_that_the_current_process_may_continue() -> Result<()> {
        // Ok(true) keeps the GUI/updater running instead of exiting silently,
        // which is the only sane Linux behaviour (there is no relaunched copy).
        assert!(ensure_admin(&[OsString::from("RTXManager")])?);
        assert!(ensure_admin(&[])?);
        Ok(())
    }
}
