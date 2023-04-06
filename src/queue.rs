use tracing::warn;

use crate::message::TransportMessage;

pub(crate) struct MessageQueue {
    next_expected_nonce: u64,
    queue: Vec<TransportMessage>,
}

impl MessageQueue {
    pub(crate) fn new() -> Self {
        MessageQueue {
            next_expected_nonce: 0,
            queue: Vec::new(),
        }
    }

    pub(crate) fn set_connection_message_received(&mut self) {
        if self.next_expected_nonce != 0 {
            panic!("connection message received twice");
        }

        self.next_expected_nonce += 1;
    }

    /// tries to push a message into the queue.
    /// if the message has the next expected nonce, then the message is returned,
    /// and should be processed by the caller.
    /// in that case, the internal queue's next expected nonce is incremented.
    pub(crate) fn try_push(&mut self, msg: TransportMessage) -> Option<TransportMessage> {
        if msg.nonce == self.next_expected_nonce {
            self.next_expected_nonce += 1;
            Some(msg)
        } else {
            if msg.nonce < self.next_expected_nonce {
                warn!("received a message with a nonce that is too low");
                return None;
            }
            self.queue.push(msg);
            self.queue.sort();
            // would probably be better to check for duplicates before insertion?
            // also would allow us to log that we received duplicates
            self.queue.dedup();
            None
        }
    }

    pub(crate) fn pop(&mut self) -> Option<TransportMessage> {
        let Some(head) = self.queue.first() else {
            return None;
        };

        if head.nonce == self.next_expected_nonce {
            self.next_expected_nonce += 1;
            Some(self.queue.pop().unwrap())
        } else {
            None
        }
    }
}
