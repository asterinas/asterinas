// SPDX-License-Identifier: MPL-2.0

//! Modeset configuration state for KMS objects.
//!
//! This module describes how planes, CRTCs, encoders, and connectors are configured
//! to form display pipelines.
//! It records plane framebuffer bindings and geometry, CRTC display modes and activation,
//! and the routing between display objects.

use alloc::boxed::Box;

use crate::kms::{
    modeset::{
        connector::DrmConnectorState, crtc::DrmCrtcState, encoder::DrmEncoderState,
        plane::DrmPlaneState,
    },
    objects::{KmsObjectIndex, KmsStaticObjects},
};

pub(crate) mod connector;
pub(crate) mod crtc;
pub(crate) mod encoder;
pub(crate) mod plane;

/// A modeset configuration indexed by per-type KMS object indices.
///
/// Getters and setters require an index from the corresponding object type
/// in the collection used to construct this state.
/// An index outside the corresponding state array causes a panic.
#[derive(Debug)]
pub(crate) struct KmsModesetState {
    plane_state: Box<[DrmPlaneState]>,
    crtc_state: Box<[DrmCrtcState]>,
    encoder_state: Box<[DrmEncoderState]>,
    connector_state: Box<[DrmConnectorState]>,
}

impl KmsModesetState {
    /// Creates default modeset states for the given KMS objects.
    pub(super) fn from_objects(objects: &KmsStaticObjects) -> Self {
        Self {
            plane_state: objects.planes().map(|_| DrmPlaneState::default()).collect(),
            crtc_state: objects.crtcs().map(|_| DrmCrtcState::default()).collect(),
            encoder_state: objects
                .encoders()
                .map(|_| DrmEncoderState::default())
                .collect(),
            connector_state: objects
                .connectors()
                .map(|_| DrmConnectorState::default())
                .collect(),
        }
    }

    pub(crate) fn plane_state(&self, index: KmsObjectIndex) -> &DrmPlaneState {
        &self.plane_state[index.get()]
    }

    pub(crate) fn crtc_state(&self, index: KmsObjectIndex) -> &DrmCrtcState {
        &self.crtc_state[index.get()]
    }

    pub(crate) fn encoder_state(&self, index: KmsObjectIndex) -> &DrmEncoderState {
        &self.encoder_state[index.get()]
    }

    pub(crate) fn connector_state(&self, index: KmsObjectIndex) -> &DrmConnectorState {
        &self.connector_state[index.get()]
    }

    pub(crate) fn set_plane_state(&mut self, index: KmsObjectIndex, state: DrmPlaneState) {
        self.plane_state[index.get()] = state;
    }

    pub(crate) fn set_crtc_state(&mut self, index: KmsObjectIndex, state: DrmCrtcState) {
        self.crtc_state[index.get()] = state;
    }

    pub(crate) fn set_encoder_state(&mut self, index: KmsObjectIndex, state: DrmEncoderState) {
        self.encoder_state[index.get()] = state;
    }

    pub(crate) fn set_connector_state(&mut self, index: KmsObjectIndex, state: DrmConnectorState) {
        self.connector_state[index.get()] = state;
    }
}
