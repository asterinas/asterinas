//! The process status

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    /// Can be scheduled to run
    Runnable,
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
}
