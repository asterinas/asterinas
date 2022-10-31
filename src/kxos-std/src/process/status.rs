//! The process status

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Can be scheduled to run
    Runnable,
    /// Suspend until be woken by SIGCONT signal
    SuspendSignalable,
    /// Exit while not reaped by parent
    Zombie,
}

impl ProcessStatus {
    pub fn set_zombie(&mut self) {
        *self = ProcessStatus::Zombie;
    }

    pub fn is_zombie(&self) -> bool {
        *self == ProcessStatus::Zombie
    }

    pub fn set_suspend(&mut self) {
        *self = ProcessStatus::SuspendSignalable;
    }

    pub fn is_suspend(&self) -> bool {
        *self == ProcessStatus::SuspendSignalable
    }

    pub fn set_runnable(&mut self) {
        *self = ProcessStatus::Runnable;
    }

    pub fn is_runnable(&self) -> bool {
        *self == ProcessStatus::Runnable
    }
}
