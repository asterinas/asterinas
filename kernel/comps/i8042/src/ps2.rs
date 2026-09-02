// SPDX-License-Identifier: MPL-2.0

//! Common utilities for PS/2 devices.

use crate::controller::{I8042Controller, I8042ControllerError};

const PS2_CMD_RESET: u8 = 0xFF;
const PS2_BAT_OK: u8 = 0xAA;

const PS2_ACK: u8 = 0xFA;
const PS2_NAK: u8 = 0xFE;
const PS2_ERR: u8 = 0xFC;
const PS2_RESULTS: &[u8] = &[PS2_ACK, PS2_NAK, PS2_ERR];

/// PS/2 device commands.
pub(super) trait Command {
    const CMD_BYTE: u8;
    const DATA_LEN: usize;
    const RES_LEN: usize;
}

macro_rules! define_commands {
    (
        $(
            $name:ident, $cmd:literal, fn([u8; $dlen:literal]) -> [u8; $rlen:literal];
        )*
    ) => {
        $(
            pub(super) struct $name;
            impl Command for $name {
                const CMD_BYTE: u8 = $cmd;
                const DATA_LEN: usize = $dlen;
                const RES_LEN: usize = $rlen;
            }
        )*
    };
}
pub(super) use define_commands;

/// Context to perform PS/2 commands.
pub(super) trait CommandCtx {
    fn controller(&mut self) -> &mut I8042Controller;

    fn write_to_port(&mut self, data: u8) -> Result<(), I8042ControllerError>;

    fn reset(&mut self) -> Result<Option<u8>, I8042ControllerError> {
        // Reset the device by sending `PS2_CMD_RESET` (reset command, supported by all PS/2
        // devices).
        self.write_to_port(PS2_CMD_RESET)?;

        let controller = self.controller();

        // The response should be `PS2_ACK` and `PS2_BAT_OK`, followed by the device PS/2 ID.
        if controller.wait_for_specific_data(PS2_RESULTS)? != PS2_ACK {
            return Err(I8042ControllerError::DeviceResetFailed);
        }
        // The reset command may take some time to finish. So we use `wait_long_and_recv_data`.
        if controller.wait_long_and_recv_data()? != PS2_BAT_OK {
            return Err(I8042ControllerError::DeviceResetFailed);
        }
        // Some keyboards won't reply its device ID. So we don't report any error here.
        Ok(controller.wait_and_recv_data().ok())
    }

    fn command<C: Command>(
        &mut self,
        args: &[u8],
        out: &mut [u8],
    ) -> Result<(), I8042ControllerError> {
        assert_eq!(args.len(), C::DATA_LEN);
        assert_eq!(out.len(), C::RES_LEN);

        // Send the command.
        self.write_to_port(C::CMD_BYTE)?;
        if self.controller().wait_for_specific_data(PS2_RESULTS)? != PS2_ACK {
            return Err(I8042ControllerError::DeviceResetFailed);
        }

        // Send the arguments.
        for &arg in args {
            self.write_to_port(arg)?;
            if self.controller().wait_for_specific_data(PS2_RESULTS)? != PS2_ACK {
                return Err(I8042ControllerError::DeviceResetFailed);
            }
        }

        // Receive the response.
        for slot in out.iter_mut() {
            *slot = self.controller().wait_and_recv_data()?;
        }

        Ok(())
    }
}
