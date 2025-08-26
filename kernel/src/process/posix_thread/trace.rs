// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    thread::{Thread, Tid},
};

#[derive(Debug)]
pub struct TraceeStatus {
    tracer: Weak<Thread>,
    tracer_tid: Tid,
}

impl TraceeStatus {
    pub fn new(tracer: Weak<Thread>, tracer_tid: Tid) -> Self {
        Self { tracer, tracer_tid }
    }

    pub fn tracer(&self) -> &Weak<Thread> {
        &self.tracer
    }

    pub fn tracer_tid(&self) -> Tid {
        self.tracer_tid
    }
}
