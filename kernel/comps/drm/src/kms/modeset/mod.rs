// SPDX-License-Identifier: MPL-2.0

//! Modeset configuration state for KMS objects.
//!
//! This module describes how planes, CRTCs, encoders, and connectors are configured
//! to form display pipelines.
//! It records plane framebuffer bindings and geometry, CRTC display modes and activation,
//! and the routing between display objects.
//!
//! `DrmModeConfig` owns and protects the aggregate state,
//! providing a common synchronization boundary for configuration changes
//! that affect multiple objects.

use alloc::boxed::Box;
use core::mem;

use crate::kms::{
    modeset::{
        connector_state::DrmConnectorState, crtc_state::DrmCrtcState,
        encoder_state::DrmEncoderState, plane_state::DrmPlaneState,
    },
    objects::{DrmKmsObjectIndex, DrmKmsObjectTopology},
};

pub(crate) mod connector_state;
pub(crate) mod crtc_state;
pub(crate) mod encoder_state;
pub(crate) mod plane_state;

/// A modeset configuration indexed by per-type KMS object indices.
///
/// State access requires an index of the corresponding object type from the topology
/// used to construct this configuration.
#[derive(Debug)]
pub(crate) struct DrmModesetState {
    plane_state: Box<[DrmPlaneState]>,
    crtc_state: Box<[DrmCrtcState]>,
    encoder_state: Box<[DrmEncoderState]>,
    connector_state: Box<[DrmConnectorState]>,
}

impl DrmModesetState {
    /// Creates default modeset states for the topology's objects.
    pub(crate) fn from_topology(topology: &DrmKmsObjectTopology) -> Self {
        // Object indices are dense within each type. All entries of a type are
        // identical initially, so the topology's iteration order is irrelevant.
        Self {
            plane_state: topology
                .planes()
                .map(|_| DrmPlaneState::default())
                .collect(),
            crtc_state: topology.crtcs().map(|_| DrmCrtcState::default()).collect(),
            encoder_state: topology
                .encoders()
                .map(|_| DrmEncoderState::default())
                .collect(),
            connector_state: topology
                .connectors()
                .map(|_| DrmConnectorState::default())
                .collect(),
        }
    }

    /// The index must identify a plane in the topology used to construct this configuration.
    /// The same requirement and out-of-bounds behavior apply to the other state accessors.
    pub(crate) fn plane_state(&self, index: DrmKmsObjectIndex) -> &DrmPlaneState {
        &self.plane_state[index.get()]
    }

    pub(crate) fn crtc_state(&self, index: DrmKmsObjectIndex) -> &DrmCrtcState {
        &self.crtc_state[index.get()]
    }

    pub(crate) fn encoder_state(&self, index: DrmKmsObjectIndex) -> &DrmEncoderState {
        &self.encoder_state[index.get()]
    }

    pub(crate) fn connector_state(&self, index: DrmKmsObjectIndex) -> &DrmConnectorState {
        &self.connector_state[index.get()]
    }

    pub(crate) fn replace_plane_state(
        &mut self,
        index: DrmKmsObjectIndex,
        state: DrmPlaneState,
    ) -> DrmPlaneState {
        mem::replace(&mut self.plane_state[index.get()], state)
    }

    pub(crate) fn replace_crtc_state(
        &mut self,
        index: DrmKmsObjectIndex,
        state: DrmCrtcState,
    ) -> DrmCrtcState {
        mem::replace(&mut self.crtc_state[index.get()], state)
    }

    pub(crate) fn replace_encoder_state(
        &mut self,
        index: DrmKmsObjectIndex,
        state: DrmEncoderState,
    ) -> DrmEncoderState {
        mem::replace(&mut self.encoder_state[index.get()], state)
    }

    pub(crate) fn replace_connector_state(
        &mut self,
        index: DrmKmsObjectIndex,
        state: DrmConnectorState,
    ) -> DrmConnectorState {
        mem::replace(&mut self.connector_state[index.get()], state)
    }
}
