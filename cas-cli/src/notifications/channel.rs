//! Channel-based notifier implementation

use std::sync::mpsc;

use crate::notifications::{NotificationEvent, Notifier};

/// Notifier that sends events through an mpsc channel
pub struct ChannelNotifier {
    sender: mpsc::Sender<NotificationEvent>,
}

/// Receiver for notification events
pub type NotificationReceiver = mpsc::Receiver<NotificationEvent>;

impl ChannelNotifier {
    /// Create a new channel notifier and its receiver
    pub fn new() -> (Self, NotificationReceiver) {
        let (sender, receiver) = mpsc::channel();
        (Self { sender }, receiver)
    }
}

impl Default for ChannelNotifier {
    fn default() -> Self {
        Self::new().0
    }
}

impl Notifier for ChannelNotifier {
    fn notify(&self, event: NotificationEvent) {
        // Best-effort send - don't block or panic if receiver is gone
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use crate::notifications::NotificationEventType;
    use crate::notifications::channel::*;

    #[test]
    fn test_channel_notifier() {
        let (notifier, receiver) = ChannelNotifier::new();

        let event =
            NotificationEvent::new(NotificationEventType::TaskCreated, "task-123", "Test task");

        notifier.notify(event.clone());

        let received = receiver.try_recv().expect("Should receive event");
        assert_eq!(received.entity_id, "task-123");
        assert_eq!(received.message, "Test task");
    }

    #[test]
    fn test_channel_notifier_dropped_receiver() {
        let (notifier, receiver) = ChannelNotifier::new();
        drop(receiver);

        // Should not panic when receiver is dropped
        let event =
            NotificationEvent::new(NotificationEventType::TaskCreated, "task-123", "Test task");
        notifier.notify(event);
    }
}
