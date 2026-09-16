use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChatTurn {
    pub user: String,
    pub assistant: String,
}

/// Bounded model context. Persistence is owned by the storage layer.
#[derive(Default, Clone)]
pub struct Conversation {
    identity: Option<(String, String)>,
    turns: Vec<ChatTurn>,
    version: u64,
}
impl Conversation {
    pub fn matches(&self, base: &str, model: &str) -> bool {
        self.identity
            .as_ref()
            .is_some_and(|(b, m)| b == base && m == model)
    }
    pub fn restore(&mut self, base: &str, model: &str, turns: Vec<ChatTurn>) {
        self.select(base, model);
        self.turns = turns;
    }
    pub fn select(&mut self, base: &str, model: &str) {
        let identity = (base.to_owned(), model.to_owned());
        if self.identity.as_ref() != Some(&identity) {
            self.clear();
            self.identity = Some(identity);
        }
    }
    pub fn history(&self) -> Vec<ChatTurn> {
        self.turns.clone()
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn clear(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.turns.clear();
    }
    pub fn complete(
        &mut self,
        base: &str,
        model: &str,
        version: u64,
        user: String,
        assistant: String,
    ) {
        if self.version != version
            || self.identity.as_ref() != Some(&(base.to_owned(), model.to_owned()))
        {
            return; // A configuration change invalidated this in-flight context.
        }
        self.turns.push(ChatTurn { user, assistant });
        while self.turns.len() > 6
            || self
                .turns
                .iter()
                .map(|t| t.user.chars().count() + t.assistant.chars().count())
                .sum::<usize>()
                > 12000
        {
            self.turns.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_is_bounded_and_can_be_cleared() {
        let mut c = Conversation::default();
        c.select("https://a.example/v1", "model-a");
        for n in 0..8 {
            c.complete(
                "https://a.example/v1",
                "model-a",
                c.version(),
                n.to_string(),
                "好".into(),
            );
        }
        assert_eq!(c.history().len(), 6);
        assert_eq!(c.history()[0].user, "2");
        c.complete(
            "https://a.example/v1",
            "model-a",
            c.version(),
            "large".into(),
            "字".repeat(11995),
        );
        assert_eq!(c.history().len(), 1);
        c.clear();
        assert!(c.history().is_empty());
    }
    #[test]
    fn switching_away_and_back_does_not_revive_old_inflight_context() {
        let mut c = Conversation::default();
        c.select("https://a.example/v1", "a");
        let version = c.version();
        c.select("https://a.example/v1", "b");
        c.select("https://a.example/v1", "a");
        c.complete(
            "https://a.example/v1",
            "a",
            version,
            "old".into(),
            "reply".into(),
        );
        assert!(c.history().is_empty());
    }
    #[test]
    fn provider_or_model_change_clears_and_rejects_late_completion() {
        let mut c = Conversation::default();
        c.select("https://a.example/v1", "a");
        c.complete(
            "https://a.example/v1",
            "a",
            c.version(),
            "old".into(),
            "reply".into(),
        );
        c.select("https://b.example/v1", "a");
        c.complete(
            "https://a.example/v1",
            "a",
            c.version(),
            "late".into(),
            "reply".into(),
        );
        assert!(c.history().is_empty());
        c.complete(
            "https://b.example/v1",
            "a",
            c.version(),
            "new".into(),
            "reply".into(),
        );
        c.select("https://b.example/v1", "b");
        assert!(c.history().is_empty());
    }
}
