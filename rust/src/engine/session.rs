use std::time::{Duration, Instant};

use uuid::Uuid;

/// Placeholder assistant message when only audio is returned by the cloud provider.
pub const CLOUD_MESSAGE_PLACEHOLDER: &str =
    "<message could not be saved due to platform limitations>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionManager {
    id: String,
    history: Vec<ChatMessage>,
    last_message_at: Option<Instant>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            history: Vec::new(),
            last_message_at: None,
        }
    }

    pub fn start_new(&mut self) {
        self.id = Uuid::new_v4().to_string();
        self.history.clear();
        self.last_message_at = None;
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn history(&self) -> &[ChatMessage] {
        &self.history
    }

    pub fn add_user_message(&mut self, text: impl Into<String>) {
        self.add_user_message_at(text, Instant::now());
    }

    pub fn add_assistant_message(&mut self, text: impl Into<String>) {
        self.add_assistant_message_at(text, Instant::now());
    }

    pub fn add_assistant_placeholder(&mut self) {
        self.add_assistant_message(CLOUD_MESSAGE_PLACEHOLDER);
    }

    pub fn add_user_message_at(&mut self, text: impl Into<String>, now: Instant) {
        self.history
            .push(ChatMessage::new(ChatRole::User, text));
        self.last_message_at = Some(now);
    }

    pub fn add_assistant_message_at(&mut self, text: impl Into<String>, now: Instant) {
        self.history
            .push(ChatMessage::new(ChatRole::Assistant, text));
        self.last_message_at = Some(now);
    }

    pub fn maybe_rollover(&mut self, timeout: Duration) -> bool {
        self.maybe_rollover_at(Instant::now(), timeout)
    }

    pub fn maybe_rollover_at(&mut self, now: Instant, timeout: Duration) -> bool {
        if let Some(last) = self.last_message_at {
            if now.duration_since(last) >= timeout {
                self.start_new();
                return true;
            }
        }
        false
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_starts_with_unique_id() {
        let session = SessionManager::new();
        assert!(!session.id().is_empty());
    }

    #[test]
    fn session_reset_creates_new_id_and_clears_history() {
        let mut session = SessionManager::new();
        session.add_user_message("hello");
        let first_id = session.id().to_string();
        session.start_new();
        assert_ne!(first_id, session.id());
        assert!(session.history().is_empty());
    }

    #[test]
    fn session_records_user_and_assistant_messages() {
        let mut session = SessionManager::new();
        session.add_user_message("hi");
        session.add_assistant_message("hello");
        assert_eq!(session.history().len(), 2);
        assert_eq!(session.history()[0].role, ChatRole::User);
        assert_eq!(session.history()[1].role, ChatRole::Assistant);
    }

    #[test]
    fn session_records_placeholder_for_cloud() {
        let mut session = SessionManager::new();
        session.add_assistant_placeholder();
        assert_eq!(session.history().len(), 1);
        assert_eq!(session.history()[0].content, CLOUD_MESSAGE_PLACEHOLDER);
    }

    #[test]
    fn session_rolls_over_after_timeout() {
        let mut session = SessionManager::new();
        let now = Instant::now();
        session.add_user_message_at("hello", now);
        let first_id = session.id().to_string();
        let rolled = session.maybe_rollover_at(now + Duration::from_secs(61), Duration::from_secs(60));
        assert!(rolled);
        assert_ne!(first_id, session.id());
        assert!(session.history().is_empty());
    }
}
