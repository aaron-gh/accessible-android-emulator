//! Folding and unfolding a foldable device (Profile::Foldable).
//!
//! The emulator's console `fold` and `unfold` set the hinge, so Android's
//! device state becomes closed or open. Without its window, the emulator
//! doesn't also switch to the folded part of the screen, so AAE sets the
//! screen size to it with `wm size`, which apps see as a screen change.

use crate::adb::Adb;
use crate::error::{Error, Result};

/// The folded screen, the left part of the unfolded one (see device.rs).
const FOLDED_SIZE: &str = "884x2208";
/// Android's device states for the emulator's foldable.
const CLOSED: &str = "1";

/// Folds or unfolds the device. Returns a message.
pub async fn set_folded(adb: &Adb, folded: bool) -> Result<String> {
    let reply = adb.raw(&["emu", if folded { "fold" } else { "unfold" }]).await?;
    if !reply.lines().any(|l| l.trim() == "OK") {
        return Err(Error::Message("This device isn't foldable.".into()));
    }
    match_screen(adb, folded).await?;
    Ok(if folded { "Folded." } else { "Unfolded." }.into())
}

/// Whether the device is folded, as Android's device state says.
pub async fn is_folded(adb: &Adb) -> Result<bool> {
    Ok(adb.shell("cmd device_state print-state").await?.trim() == CLOSED)
}

/// Sets the screen size to match the fold, as after a start: a cold boot
/// starts unfolded, and a size set while folded would otherwise stay.
pub async fn match_screen(adb: &Adb, folded: bool) -> Result<()> {
    let command = if folded {
        format!("wm size {FOLDED_SIZE}")
    } else {
        "wm size reset".to_string()
    };
    adb.shell(&command).await?;
    Ok(())
}
