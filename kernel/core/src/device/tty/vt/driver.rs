// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use aster_framebuffer::{
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode},
};
use ostd::mm::VmIo;

use crate::{
    context::current_userspace,
    device::{
        Device, DevtmpfsInodeMeta,
        tty::{
            CFontOp, Tty, TtyDriver,
            termio::CTermios,
            vt::{
                VtIndex,
                c_types::{CVtMode, ReleaseDisplayType},
                file::{VtFile, VtOpenFileCounter},
                manager::{VT_MANAGER, VtConsole, VtMode},
            },
        },
    },
    fs::file::PerOpenFileOps,
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

/// The driver for VT (virtual terminal) devices.
pub struct VtDriver {
    index: VtIndex,
    console: VtConsole,
    open_file_counter: VtOpenFileCounter,
}

impl VtDriver {
    fn handle_set_font(&self, font_op: &CFontOp) -> Result<()> {
        let CFontOp {
            op,
            flags: _,
            width,
            height,
            charcount,
            data,
            ..
        } = font_op;

        let vpitch = match *op {
            CFontOp::OP_SET => CFontOp::NONTALL_VPITCH,
            CFontOp::OP_SET_TALL => font_op.height,
            CFontOp::OP_SET_DEFAULT => {
                return self.console.set_font(BitmapFont::new_basic8x8());
            }
            _ => return_errno_with_message!(Errno::EINVAL, "the font operation is invalid"),
        };

        if *width == 0
            || *height == 0
            || *width > CFontOp::MAX_WIDTH
            || *height > CFontOp::MAX_HEIGHT
            || *charcount > CFontOp::MAX_CHARCOUNT
            || *height > vpitch
        {
            return_errno_with_message!(Errno::EINVAL, "the font is invalid or too large");
        }

        let font_size = width.div_ceil(u8::BITS) * vpitch * charcount;
        let mut font_data = vec![0; font_size as usize];
        current_userspace!().read_bytes(*data as Vaddr, &mut font_data[..])?;

        // In Linux, the most significant bit represents the first pixel, but `BitmapFont` requires
        // the least significant bit to represent the first pixel. So now we reverse the bits.
        font_data
            .iter_mut()
            .for_each(|byte| *byte = byte.reverse_bits());

        let font = BitmapFont::new_with_vpitch(
            *width as usize,
            *height as usize,
            vpitch as usize,
            font_data,
        );
        self.console.set_font(font)?;

        Ok(())
    }
}

impl TtyDriver for VtDriver {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/linux/major.h#L18>.
    const DEVICE_MAJOR_ID: u32 = 4;

    fn devtmpfs_meta(&self, index: u32) -> Option<DevtmpfsInodeMeta<'_>> {
        Some(DevtmpfsInodeMeta::new(format!("tty{}", index)))
    }

    fn open(tty: Arc<Tty<Self>>) -> Result<Box<dyn PerOpenFileOps>> {
        Ok(Box::new(VtFile::new(tty)?))
    }

    fn push_output(&self, chs: &[u8]) -> Result<usize> {
        self.console.send(chs);
        Ok(chs.len())
    }

    fn echo_callback(&self) -> impl FnMut(&[u8]) + '_ {
        |chs| self.console.send(chs)
    }

    fn can_push(&self) -> bool {
        true
    }

    fn notify_input(&self) {}

    fn on_termios_change(&self, _old_termios: &CTermios, _new_termios: &CTermios) {}

    fn ioctl(&self, tty: &Tty<Self>, raw_ioctl: RawIoctl) -> Result<bool>
    where
        Self: Sized,
    {
        use super::ioctl_defs::*;

        dispatch_ioctl!(match raw_ioctl {
            cmd @ GetKeyboardType => {
                // Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/kd.h#L36>.
                const KB_101: u8 = 0x02;

                cmd.write(&KB_101)?;
            }
            cmd @ SetGraphicsMode => {
                check_vt_ioctl_perm(tty)?;

                let mode = ConsoleMode::try_from(cmd.get())?;
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let guard = vtm.lock();

                guard.set_console_mode(self.index, mode);
            }
            cmd @ GetGraphicsMode => {
                let mode = self.console.mode();

                cmd.write(&(mode as i32))?;
            }
            cmd @ GetKeyboardMode => {
                let mode = self.console.lock_keyboard().mode();

                cmd.write(&(mode as i32))?;
            }
            cmd @ SetKeyboardMode => {
                check_vt_ioctl_perm(tty)?;

                let mode = KeyboardMode::try_from(cmd.get())?;

                self.console.lock_keyboard().set_mode(mode)?;
            }
            cmd @ SetOrGetFont => {
                let font_op = cmd.read()?;

                if font_op.op == CFontOp::OP_GET {
                    // TODO: Add support for getting the font of the console device.
                    return_errno_with_message!(Errno::EINVAL, "getting font data is not supported");
                }

                check_vt_ioctl_perm(tty)?;

                self.handle_set_font(&font_op)?;
            }
            cmd @ GetAvailableVt => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let res = {
                    let guard = vtm.lock();
                    guard
                        .get_available_vt_index()
                        .map(|index| index.get() as i32)
                        .unwrap_or(-1)
                };

                cmd.write(&res)?;
            }
            cmd @ GetVtMode => {
                let c_vt_mode = {
                    let vt_console_inner = self.console.lock_inner();
                    (*vt_console_inner.vt_mode()).into()
                };

                cmd.write(&c_vt_mode)?;
            }
            cmd @ SetVtMode => {
                check_vt_ioctl_perm(tty)?;

                let c_vt_mode: CVtMode = cmd.read()?;
                let vt_mode: VtMode = c_vt_mode.try_into()?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let mut guard = vtm.lock();
                guard.set_vt_mode(self.index, vt_mode);
            }
            cmd @ GetVtState => {
                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let state = {
                    let guard = vtm.lock();
                    guard.get_global_vt_state()
                };

                cmd.write(&state)?;
            }
            cmd @ ReleaseDisplay => {
                check_vt_ioctl_perm(tty)?;

                let reldisp_type = ReleaseDisplayType::from(cmd.get());

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let mut guard = vtm.lock();
                guard.handle_release_display(self.index, reldisp_type)?;
            }
            cmd @ ActivateVt => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L835-L854>

                check_vt_ioctl_perm(tty)?;

                let vt_index = VtIndex::try_from(cmd.get())?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                let mut guard = vtm.lock();
                guard.allocate_vt(vt_index);
                // Linux ignores `set_console` return semantics in `VT_ACTIVATE`.
                let _ = guard.switch_vt(vt_index);
            }
            cmd @ WaitForVtActive => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L862-L870>

                check_vt_ioctl_perm(tty)?;

                let vt_index = VtIndex::try_from(cmd.get())?;

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");
                vtm.wait_for_vt_active(vt_index)?;
            }
            cmd @ DisallocateVt => {
                // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L893-L906>

                let vtm = VT_MANAGER.get().expect("`VT_MANAGER` is not initialized");

                if cmd.get() == 0 {
                    let guard = vtm.lock();
                    guard.disallocate_all_vts();
                } else {
                    let vt_index = VtIndex::try_from(cmd.get())?;

                    let guard = vtm.lock();
                    guard.disallocate_vt(vt_index)?;
                }
            }
            _ => return Ok(false),
        });

        Ok(true)
    }
}

fn check_vt_ioctl_perm(tty: &Tty<VtDriver>) -> Result<()> {
    // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/vt/vt_ioctl.c#L743-L749>

    if current!()
        .terminal()
        .is_some_and(|terminal| terminal.id() == tty.id())
    {
        return Ok(());
    }

    let init_user_ns = UserNamespace::get_init_singleton();
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        init_user_ns.as_ref(),
        current_thread!().as_posix_thread().unwrap(),
        CapSet::SYS_TTY_CONFIG,
    ))?;

    Ok(())
}

impl VtDriver {
    /// Creates a new VT driver with none VT console backend.
    pub(super) fn new(index: VtIndex) -> Self {
        Self {
            index,
            console: VtConsole::new(),
            open_file_counter: VtOpenFileCounter::new(),
        }
    }

    /// Gets the VT console associated with this driver.
    pub(super) fn vt_console(&self) -> &VtConsole {
        &self.console
    }

    /// Gets the VT index associated with this driver.
    pub(super) fn index(&self) -> VtIndex {
        self.index
    }

    /// Gets the open file counter of this VT driver.
    pub(super) fn open_file_counter(&self) -> &VtOpenFileCounter {
        &self.open_file_counter
    }
}
