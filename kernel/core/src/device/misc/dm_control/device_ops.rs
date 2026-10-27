// SPDX-License-Identifier: MPL-2.0

//! Typed tableless lifecycle operations behind the Linux DM ioctl ABI.
//!
//! Mutation callers retain a checked lifecycle guard through response encoding.
//! Queries capture owned identities before producing userspace records.

use aster_device_mapper::{DmControlRef, DmDevice, DmManager};
use device_id::DeviceId;

use super::{
    abi::{map_create_error, map_dm_error},
    commands::CurrentDevice,
};
use crate::prelude::*;

/// Decoded parameters for `DM_DEV_CREATE`.
pub(super) struct CreateRequest {
    pub(super) name: String,
    pub(super) uuid: Option<Vec<u8>>,
    pub(super) requested_minor: Option<u32>,
}

/// Immutable identity needed to encode a tableless device response.
pub(super) struct DeviceSnapshot {
    pub(super) id: DeviceId,
    pub(super) name: String,
    pub(super) uuid: Option<Vec<u8>>,
}

/// Captures immutable query state before ABI record encoding.
pub(super) struct QueryWorkflow;

impl QueryWorkflow {
    pub(super) fn device(device: &DmDevice) -> DeviceSnapshot {
        DeviceSnapshot {
            id: device.id(),
            name: device.mapper_name(),
            uuid: device.uuid(),
        }
    }
}

/// Coordinates typed control-plane mutations for one manager.
pub(super) struct ControlWorkflow<'a> {
    manager: &'a DmManager,
}

impl<'a> ControlWorkflow<'a> {
    pub(super) fn new(manager: &'a DmManager) -> Self {
        Self { manager }
    }

    pub(super) fn create(&self, request: CreateRequest) -> Result<DmControlRef> {
        self.manager
            .create_control(request.name, request.uuid, request.requested_minor)
            .map_err(map_create_error)
    }

    /// Detaches a current tableless mapper without publishing a removal event.
    pub(super) fn remove(&self, device: &CurrentDevice<'_>) -> Result<()> {
        self.manager
            .remove(&device.mapper_name())
            .map_err(map_dm_error)?;
        Ok(())
    }
}
