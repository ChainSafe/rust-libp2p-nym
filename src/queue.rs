use std::collections::{HashSet, VecDeque};
use tracing::{debug, warn};

use crate::message::TransportMessage;

/// MessageQueue is a queue of messages, ordered by nonce, that we've
/// received but are not yet able to process because we're waiting for
/// a message with the next expected nonce first.
/// This is required because Nym does not guarantee any sort of message
/// ordering, only delivery.
/// TODO: is there a DOS vector here where a malicious peer sends us
/// messages only with nonce higher than the next expected nonce?
pub(crate) struct MessageQueue {
    /// nonce of the next message we expect to receive on the
    /// connection.
    /// any messages with a nonce greater than this are pushed into
    /// the queue.
    /// if we get a message with a nonce equal to this, then we
    /// immediately handle it in the transport and increment the nonce.
    next_expected_nonce: u64,

    /// the actual queue of messages, ordered by nonce.
    /// the head of the queue's nonce is always greater
    /// than the next expected nonce.
    queue: VecDeque<TransportMessage>,

    /// tracks the nonces that exist in the queue.
    /// used to check for duplicate nonces on push.
    nonce_set: HashSet<u64>,
}

impl MessageQueue {
    pub(crate) fn new() -> Self {
        MessageQueue {
            next_expected_nonce: 0,
            queue: VecDeque::new(),
            nonce_set: HashSet::new(),
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

            if self.nonce_set.contains(&msg.nonce) {
                // this shouldn't happen normally, only if the other node
                // is not following the protocol
                warn!("received a message with a duplicate nonce");
                return None;
            }

            self.nonce_set.insert(msg.nonce);
            self.queue.push_back(msg);
            self.queue.make_contiguous().sort();
            None
        }
    }

    pub(crate) fn pop(&mut self) -> Option<TransportMessage> {
        let Some(head) = self.queue.front() else {
            return None;
        };

        if head.nonce == self.next_expected_nonce {
            self.next_expected_nonce += 1;
            self.nonce_set.remove(&head.nonce);
            Some(self.queue.pop_front().unwrap())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod test {
    use crate::message::{ConnectionId, SubstreamId, SubstreamMessage};

    use super::*;

    impl TransportMessage {
        fn new(nonce: u64, message: SubstreamMessage, id: ConnectionId) -> Self {
            TransportMessage { nonce, message, id }
        }
    }

    #[test]
    fn test_message_queue() {
        let mut queue = MessageQueue::new();

        let test_substream_message =
            SubstreamMessage::new_with_data(SubstreamId::generate(), vec![0, 1, 2]);
        let connection_id = ConnectionId::generate();

        let msg1 = TransportMessage::new(0, test_substream_message.clone(), connection_id.clone());
        let msg2 = TransportMessage::new(1, test_substream_message.clone(), connection_id.clone());
        let msg3 = TransportMessage::new(2, test_substream_message.clone(), connection_id.clone());

        assert_eq!(queue.try_push(msg1.clone()), Some(msg1));
        assert_eq!(queue.try_push(msg3.clone()), None);
        assert_eq!(queue.try_push(msg2.clone()), Some(msg2));

        assert_eq!(queue.pop(), Some(msg3));

        assert_eq!(queue.pop(), None);

        let msg4 = TransportMessage::new(3, test_substream_message.clone(), connection_id.clone());
        assert_eq!(queue.try_push(msg4.clone()), Some(msg4));

        assert_eq!(queue.pop(), None);
        assert_eq!(queue.pop(), None);
        assert_eq!(queue.pop(), None);
        assert_eq!(queue.pop(), None);
        assert_eq!(queue.next_expected_nonce, 4);

        // should just return the message and increment nonce when message nonce = next expected nonce
        let msg5 = TransportMessage::new(4, test_substream_message, connection_id);
        assert_eq!(queue.try_push(msg5.clone()), Some(msg5));
        assert_eq!(queue.next_expected_nonce, 5);
    }
}
