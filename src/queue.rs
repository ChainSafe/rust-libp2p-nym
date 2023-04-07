use std::collections::VecDeque;
use tracing::{debug, warn};

use crate::message::TransportMessage;

pub(crate) struct MessageQueue {
    next_expected_nonce: u64,
    queue: VecDeque<TransportMessage>,
}

impl MessageQueue {
    pub(crate) fn new() -> Self {
        MessageQueue {
            next_expected_nonce: 0,
            queue: VecDeque::new(),
        }
    }

    pub(crate) fn print_nonces(&self) {
        let mut nonces = "".to_string();
        self.queue.iter().for_each(|msg| {
            nonces += &msg.nonce.to_string();
            nonces += ", ";
        });
        debug!("MessageQueue: [{:?}]", nonces);
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
                // this shouldn't happen normally, only if the other node
                // is not following the protocol
                warn!("received a message with a nonce that is too low");
                return None;
            }
            self.queue.push_back(msg);
            self.queue.make_contiguous().sort();
            // TODO: we need  to check for duplicates before insertion?
            // also log that we received duplicates
            // if a node sends us messages with duplicate nonces, should we
            // disconnect as they're not following the protocol?
            // self.queue.dedup();
            None
        }
    }

    pub(crate) fn pop(&mut self) -> Option<TransportMessage> {
        let Some(head) = self.queue.front() else {
            return None;
        };

        if head.nonce == self.next_expected_nonce {
            self.next_expected_nonce += 1;
            Some(self.queue.pop_front().unwrap())
        } else {
            None
        }
    }
}
