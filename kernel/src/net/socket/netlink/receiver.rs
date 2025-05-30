// SPDX-License-Identifier: MPL-2.0

use crate::{events::IoEvents, prelude::*, process::signal::Pollee};

pub struct MessageReceiver<Message> {
    message_queue: MessageQueue<Message>,
    pollee: Pollee,
}

pub(super) struct MessageQueue<Message>(pub(super) Arc<Mutex<VecDeque<Message>>>);

impl<Message> Clone for MessageQueue<Message> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<Message> MessageQueue<Message> {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::new())))
    }

    fn enqueue(&self, message: Message) -> Result<()> {
        // FIXME: We should verify the socket buffer length to ensure
        // that adding the message doesn't exceed the buffer capacity.
        self.0.lock().push_back(message);
        Ok(())
    }
}

impl<Message> MessageReceiver<Message> {
    pub(super) const fn new(message_queue: MessageQueue<Message>, pollee: Pollee) -> Self {
        Self {
            message_queue,
            pollee,
        }
    }

    pub(super) fn enqueue_message(&self, message: Message) -> Result<()> {
        self.message_queue.enqueue(message)?;
        self.pollee.notify(IoEvents::IN);

        Ok(())
    }
}
