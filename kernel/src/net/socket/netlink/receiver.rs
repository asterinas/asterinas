// SPDX-License-Identifier: MPL-2.0

use crate::{events::IoEvents, prelude::*, process::signal::Pollee};

pub struct MessageReceiver<Message> {
    message_queue: Arc<Mutex<MessageQueue<Message>>>,
    pollee: Pollee,
}

pub(super) struct MessageQueue<Message>(VecDeque<Message>);

impl<Message> MessageQueue<Message> {
    pub(super) fn new_pair(pollee: Pollee) -> (Arc<Mutex<Self>>, MessageReceiver<Message>) {
        let queue = Arc::new(Mutex::new(Self(VecDeque::new())));
        let receiver = MessageReceiver {
            message_queue: queue.clone(),
            pollee,
        };
        (queue, receiver)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn peek(&self) -> Option<&Message> {
        self.0.front()
    }

    pub(super) fn dequeue(&mut self) -> Option<Message> {
        self.0.pop_front()
    }

    pub(self) fn enqueue(&mut self, message: Message) -> Result<()> {
        // FIXME: We should verify the socket buffer length to ensure
        // that adding the message doesn't exceed the buffer capacity.
        self.0.push_back(message);
        Ok(())
    }
}

impl<Message> MessageReceiver<Message> {
    pub(super) fn enqueue_message(&self, message: Message) -> Result<()> {
        self.message_queue.lock().enqueue(message)?;
        self.pollee.notify(IoEvents::IN);

        Ok(())
    }
}
