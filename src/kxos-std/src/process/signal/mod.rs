pub mod constants;
pub mod sig_action;
pub mod sig_disposition;
pub mod sig_mask;
pub mod sig_num;
pub mod sig_queues;
pub mod signals;

use crate::{
    prelude::*,
    process::signal::sig_action::{SigAction, SigDefaultAction},
};

/// Handle pending signal for current process
pub fn handle_pending_signal() {
    let current = current!();
    let sig_queues = current.sig_queues();
    let mut sig_queues_guard = sig_queues.lock();
    let sig_mask = current.sig_mask().lock().clone();
    if let Some(signal) = sig_queues_guard.dequeue(&sig_mask) {
        let sig_num = signal.num();
        debug!("sig_num = {:?}", sig_num);
        let sig_action = current.sig_dispositions().lock().get(sig_num);
        match sig_action {
            SigAction::Ign => {
                debug!("Ignore signal {:?}", sig_num);
            }
            SigAction::User { .. } => todo!(),
            SigAction::Dfl => {
                let sig_default_action = SigDefaultAction::from_signum(sig_num);
                match sig_default_action {
                    SigDefaultAction::Core | SigDefaultAction::Term => {
                        // FIXME: How to set correct status if process is terminated
                        current.exit(1);
                    }
                    SigDefaultAction::Ign => {}
                    SigDefaultAction::Stop => {
                        let mut status_guard = current.status().lock();
                        if status_guard.is_runnable() {
                            status_guard.set_suspend();
                        } else {
                            panic!("Try to suspend a not running process.")
                        }
                        drop(status_guard);
                    }
                    SigDefaultAction::Cont => {
                        let mut status_guard = current.status().lock();
                        if status_guard.is_suspend() {
                            status_guard.set_runnable();
                        } else {
                            panic!("Try to continue a not suspended process.")
                        }
                        drop(status_guard);
                    }
                }
            }
        }
    }
}
