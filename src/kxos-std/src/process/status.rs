#[derive(Debug, Clone, Copy)]
pub enum ProcessStatus {
    Runnable,
    Zombie,
}

impl ProcessStatus {
    pub fn set_zombie(&mut self) {
        *self = ProcessStatus::Zombie;
    }
}
