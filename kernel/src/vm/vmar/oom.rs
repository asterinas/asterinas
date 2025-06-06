// SPDX-License-Identifier: MPL-2.0

//! Out-Of-Memory (OOM) Controller.

use ostd::sync::Mutex;

use super::RssType;
use crate::{
    prelude::*,
    process::{
        process_table,
        signal::{constants::SIGKILL, signals::kernel::KernelSignal},
        Process,
    },
};

static OOM_LOCK: Mutex<()> = Mutex::new(());

/// Computes the OOM badness score for a given process, in order to
/// heuristically determine which candidate to kill.
///
/// Higher score indicates higher memory usage and higher probability
/// of being killed when we run out of memory.
///
/// Returns `None` for unkillable processes.
fn oom_badness(process: &Arc<Process>) -> Option<usize> {
    if process.is_init_process() {
        return None;
    }
    let vmar = process.lock_root_vmar();
    let rss = vmar.unwrap().get_rss_counter(RssType::RSS_ANONPAGES)
        + vmar.unwrap().get_rss_counter(RssType::RSS_FILEPAGES);

    Some(rss)
}

/// Handles an Out-Of-Memory condition by selecting and terminating
/// the process with the highest OOM badness score.
pub(super) fn out_of_memory() -> Result<()> {
    if let Some(_gurad) = OOM_LOCK.try_lock() {
        let process_table_mut = process_table::process_table_mut();
        let worst_process = process_table_mut
            .iter()
            .filter_map(|process| oom_badness(process).map(|score| (score, process)))
            .max_by_key(|&(score, _)| score)
            .map(|(_, process)| process);

        if let Some(process) = worst_process {
            warn!("OOM: killing process pid = {}", process.pid(),);
            process.enqueue_signal(KernelSignal::new(SIGKILL));
            Ok(())
        } else {
            return_errno_with_message!(
                Errno::ENOMEM,
                "The Out-Of-Memory controller failed to select a killable process."
            );
        }
    } else {
        // Someone is already handling the OOM condition.
        Ok(())
    }
}
