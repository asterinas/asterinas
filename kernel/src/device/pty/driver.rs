// SPDX-License-Identifier: MPL-2.0

use aster_console::AnyConsoleDevice;
use ostd::sync::SpinLock;

use super::file::PtySlaveFile;
use crate::{
    device::{
        pty::packet::{PacketCtrl, PacketStatus},
        tty::{
            Tty, TtyDriver, TtyFlags,
            termio::{CCtrlCharId, CInputFlags, CLocalFlags, CTermios},
        },
    },
    events::IoEvents,
    fs::file::FileIo,
    prelude::*,
    process::signal::Pollee,
    util::ring_buffer::RingBuffer,
};

const BUFFER_CAPACITY: usize = 8192;

/// A pseudoterminal driver.
///
/// This is contained in the pty slave, but it maintains the output buffer and the pollee of the
/// master. The pollee of the slave is part of the [`Tty`] structure (see the definition of
/// [`PtySlave`]).
pub struct PtyDriver {
    output: SpinLock<RingBuffer<u8>>,
    pollee: Pollee,
    opened_slaves: SpinLock<usize>,
    tty_flags: TtyFlags,
    packet_ctrl: PacketCtrl,
}

/// A pseudoterminal slave.
pub type PtySlave = Tty<PtyDriver>;

impl PtyDriver {
    pub(super) fn new() -> Self {
        let tty_flags = TtyFlags::new();
        tty_flags.set_pty_locked();
        Self {
            output: SpinLock::new(RingBuffer::new(BUFFER_CAPACITY)),
            pollee: Pollee::new(),
            opened_slaves: SpinLock::new(0),
            tty_flags,
            packet_ctrl: PacketCtrl::new(),
        }
    }

    pub(super) fn try_read(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/n_tty.c#L2245>.
        match self.packet_ctrl.take_status() {
            None => {
                // Packet mode is disabled.
                self.read_output(buf)
            }
            Some(packet_status) if !packet_status.is_empty() => {
                // Some packet status is pending.
                buf[0] = packet_status.bits();
                Ok(1)
            }
            Some(_) => {
                // There's no pending packet status.
                let data_len = self.read_output(&mut buf[1..])?;
                buf[0] = 0;
                Ok(data_len + 1)
            }
        }
    }

    fn read_output(&self, buf: &mut [u8]) -> Result<usize> {
        let mut output = self.output.lock();
        if output.is_empty() {
            if self.tty_flags.is_other_closed() {
                return_errno_with_message!(Errno::EIO, "the pty slave has been closed");
            }
            return_errno_with_message!(Errno::EAGAIN, "the buffer is empty");
        }

        let read_len = output.len().min(buf.len());
        output.pop_slice(&mut buf[..read_len]).unwrap();
        Ok(read_len)
    }

    pub(super) fn pollee(&self) -> &Pollee {
        &self.pollee
    }

    pub(super) fn buffer_len(&self) -> usize {
        self.output.lock().len()
    }

    pub(super) fn opened_slaves(&self) -> &SpinLock<usize> {
        &self.opened_slaves
    }

    pub(super) fn tty_flags(&self) -> &TtyFlags {
        &self.tty_flags
    }

    pub(super) fn packet_ctrl(&self) -> &PacketCtrl {
        &self.packet_ctrl
    }
}

impl TtyDriver for PtyDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L147>.
    const DEVICE_MAJOR_ID: u32 = 136;

    fn devtmpfs_path(&self, _index: u32) -> Option<String> {
        None
    }

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(PtySlaveFile::new(tty)?))
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
        let mut output = self.output.lock();

        let mut len = 0;
        for ch in chs {
            // TODO: This is termios-specific behavior and should be part of the TTY implementation
            // instead of the TTY driver implementation. See the ONLCR flag for more details.
            if *ch == b'\n' && output.capacity() - output.len() >= 2 {
                output.push(b'\r').unwrap();
                output.push(b'\n').unwrap();
            } else if *ch != b'\n' && !output.is_full() {
                output.push(*ch).unwrap();
            } else if len == 0 {
                return_errno_with_message!(Errno::EAGAIN, "the output buffer is full");
            } else {
                break;
            }
            len += 1;
        }

        self.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
        Ok(len)
    }

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        let mut output = self.output.lock();
        let mut has_notified = false;

        move |chs| {
            for ch in chs {
                let _ = output.push(*ch);
            }

            if !has_notified {
                self.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
                has_notified = true;
            }
        }
    }

    fn can_push(&self) -> bool {
        let output = self.output.lock();
        output.capacity() - output.len() >= 2
    }

    fn notify_input(&self) {
        self.pollee.notify(IoEvents::OUT);
    }

    fn console(&self) -> Option<&dyn AnyConsoleDevice> {
        None
    }

    fn on_termios_change(&self, old_termios: &CTermios, new_termios: &CTermios) {
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/pty.c#L246>.
        let extproc = old_termios.local_flags().contains(CLocalFlags::EXTPROC)
            || new_termios.local_flags().contains(CLocalFlags::EXTPROC);
        let old_flow = old_termios.input_flags().contains(CInputFlags::IXON)
            && old_termios.special_char(CCtrlCharId::VSTOP) == 0o23
            && old_termios.special_char(CCtrlCharId::VSTART) == 0o21;
        let new_flow = new_termios.input_flags().contains(CInputFlags::IXON)
            && new_termios.special_char(CCtrlCharId::VSTOP) == 0o23
            && new_termios.special_char(CCtrlCharId::VSTART) == 0o21;

        if (old_flow == new_flow) && !extproc {
            return;
        }

        let has_set = self.packet_ctrl.set_status(|packet_status| {
            if old_flow != new_flow {
                *packet_status &= !(PacketStatus::DOSTOP | PacketStatus::NOSTOP);
                if new_flow {
                    *packet_status |= PacketStatus::DOSTOP;
                } else {
                    *packet_status |= PacketStatus::NOSTOP;
                }
            }

            if extproc {
                *packet_status |= PacketStatus::IOCTL;
            }
        });

        if has_set {
            self.pollee
                .notify(IoEvents::PRI | IoEvents::IN | IoEvents::RDNORM);
        }
    }
}
