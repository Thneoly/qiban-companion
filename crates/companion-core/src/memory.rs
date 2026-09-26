//! Explicit, user-confirmed memory. No extraction, model calls or persistence here.
use serde::{Deserialize, Serialize};

pub const MAX_MEMORIES: usize = 30;
pub const MAX_BODY_CHARS: usize = 200;
pub const MAX_SELECTION: usize = 5;
pub const MAX_CONTEXT_CHARS: usize = 800;
// Exact in SQLite INTEGER and JavaScript number; fail instead of losing precision.
pub const MAX_COUNTER: i64 = 9_007_199_254_740_991;
pub const MANUAL_SOURCE_LABEL: &str = "用户在记忆面板填写";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemoryError {
    #[error("记忆内容或日期无效")]
    InvalidInput,
    #[error("不能手动创建任务事实")]
    UnsupportedKind,
    #[error("最多保留30条有效记忆")]
    CapacityExceeded,
    #[error("最多选择5条、合计800字")]
    SelectionTooLarge,
    #[error("记忆版本已变化，请刷新")]
    Conflict,
    #[error("上下文已变化，请刷新")]
    ContextChanged,
    #[error("版本计数已达到上限")]
    CounterExhausted,
    #[error("记忆不存在或已删除")]
    NotFound,
    #[error("请先确认清空全部聊天")]
    ConfirmationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryScope {
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPolicy {
    pub enabled: bool,
    pub revision: i64,
    pub selected_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryPolicyChange {
    pub expected_scope: MemoryScope,
    pub expected_revision: i64,
    pub expected_epoch: i64,
    pub enabled: bool,
    pub selected_ids: Vec<String>,
    pub restart_conversation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    Experience,
    TaskFact,
}
impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Experience => "experience",
            Self::TaskFact => "task_fact",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    UserManual,
}

/// User-editable fields only. IDs, provenance, timestamps and versions are host-owned.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemoryDraft {
    pub kind: MemoryKind,
    pub body: String,
    pub event_date: Option<String>,
}
impl MemoryDraft {
    pub fn validate(&self) -> Result<Self, MemoryError> {
        if self.kind == MemoryKind::TaskFact {
            return Err(MemoryError::UnsupportedKind);
        }
        // Validate before trimming so an illegal trailing control cannot disappear.
        if self
            .body
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(MemoryError::InvalidInput);
        }
        let body = self.body.trim();
        if body.is_empty() || body.chars().count() > MAX_BODY_CHARS {
            return Err(MemoryError::InvalidInput);
        }
        if let Some(date) = &self.event_date {
            validate_event_date(date)?;
        }
        Ok(Self {
            kind: self.kind,
            body: body.into(),
            event_date: self.event_date.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub kind: MemoryKind,
    pub body: String,
    pub source_kind: MemorySource,
    pub source_label: String,
    pub event_date: Option<String>,
    pub created_at: i64,
    pub confirmed_at: i64,
    pub updated_at: i64,
    pub revision: i64,
}

/// Minimal internal deletion marker, deliberately has no content/provenance fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryTombstone {
    pub id: String,
    pub kind: MemoryKind,
    pub revision: i64,
    pub deleted_at: i64,
}

pub fn next_counter(value: i64) -> Result<i64, MemoryError> {
    value
        .checked_add(1)
        .filter(|_| value >= 0)
        .filter(|v| *v <= MAX_COUNTER)
        .ok_or(MemoryError::CounterExhausted)
}

pub fn validate_event_date(date: &str) -> Result<(), MemoryError> {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return Err(MemoryError::InvalidInput);
    }
    let year: u32 = date[..4].parse().map_err(|_| MemoryError::InvalidInput)?;
    let month: u32 = date[5..7].parse().map_err(|_| MemoryError::InvalidInput)?;
    let day: u32 = date[8..].parse().map_err(|_| MemoryError::InvalidInput)?;
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if year == 0 || day == 0 || day > days {
        return Err(MemoryError::InvalidInput);
    }
    Ok(())
}

/// Keeps caller's explicit order, rejects unknown/duplicate IDs and excess budgets.
pub fn validate_selection<'a>(
    ids: &[String],
    active: &'a [Memory],
) -> Result<Vec<&'a Memory>, MemoryError> {
    if ids.len() > MAX_SELECTION {
        return Err(MemoryError::SelectionTooLarge);
    }
    let mut result = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        if ids[..i].contains(id) {
            return Err(MemoryError::InvalidInput);
        }
        result.push(
            active
                .iter()
                .find(|m| &m.id == id)
                .ok_or(MemoryError::NotFound)?,
        );
    }
    if result.iter().map(|m| m.body.chars().count()).sum::<usize>() > MAX_CONTEXT_CHARS {
        return Err(MemoryError::SelectionTooLarge);
    }
    Ok(result)
}

/// Personal memories live in the standalone memory service and use integer
/// IDs with their own (id, seq) versioning; selection shares the app-memory
/// budget constants but is validated against fetched content by the host
/// (this layer cannot touch the network).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMemoryPolicy {
    pub enabled: bool,
    pub revision: i64,
    pub selected_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryChange {
    pub expected_scope: MemoryScope,
    pub expected_revision: i64,
    pub expected_epoch: i64,
    pub enabled: bool,
    pub selected_ids: Vec<i64>,
    pub restart_conversation: bool,
}

/// Order-preserving structural check only: positive integers, no duplicates,
/// at most MAX_SELECTION. Existence, activity and the character budget are
/// verified by the host against freshly fetched service content.
pub fn validate_personal_selection(ids: &[i64]) -> Result<(), MemoryError> {
    if ids.len() > MAX_SELECTION {
        return Err(MemoryError::SelectionTooLarge);
    }
    for (i, id) in ids.iter().enumerate() {
        if *id < 1 || ids[..i].contains(id) {
            return Err(MemoryError::InvalidInput);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_shared_typescript_contract_fixture() {
        let memory = Memory {
            id: "00000000-0000-4000-8000-000000000001".into(),
            kind: MemoryKind::Experience,
            body: "完成了第一次徒步🌱".into(),
            source_kind: MemorySource::UserManual,
            source_label: MANUAL_SOURCE_LABEL.into(),
            event_date: Some("2024-02-29".into()),
            created_at: 1000,
            confirmed_at: 1001,
            updated_at: 1001,
            revision: 2,
        };
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../packages/contracts/fixtures/manual-memory.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(memory).unwrap(), fixture);
    }
    #[test]
    fn validates_unicode_controls_dates_and_provenance() {
        let mut draft = MemoryDraft {
            kind: MemoryKind::Experience,
            body: " 🌱 ".into(),
            event_date: Some("2024-02-29".into()),
        };
        assert_eq!(draft.validate().unwrap().body, "🌱");
        draft.body = "🌱".repeat(200);
        assert!(draft.validate().is_ok());
        draft.body.push('🌱');
        assert_eq!(draft.validate().unwrap_err(), MemoryError::InvalidInput);
        for body in [" \t\n", "a\0", "a\r", "a\u{7f}"] {
            draft.body = body.into();
            assert!(draft.validate().is_err());
        }
        for date in [
            "2023-02-29",
            "1900-02-29",
            "2024-04-31",
            "0000-01-01",
            "2024-00-01",
            "2024-01-00",
            "2024-1-01",
            "２０２４-01-01",
        ] {
            assert!(validate_event_date(date).is_err(), "{date}");
        }
        assert!(validate_event_date("2000-02-29").is_ok());
        draft.kind = MemoryKind::TaskFact;
        assert_eq!(draft.validate().unwrap_err(), MemoryError::UnsupportedKind);
        assert!(serde_json::from_str::<MemoryDraft>(
            r#"{"kind":"preference","body":"x","sourceKind":"executor"}"#
        )
        .is_err());
    }
    #[test]
    fn counters_never_wrap_or_lose_js_precision() {
        assert_eq!(next_counter(MAX_COUNTER - 1), Ok(MAX_COUNTER));
        assert_eq!(
            next_counter(MAX_COUNTER),
            Err(MemoryError::CounterExhausted)
        );
        assert!(next_counter(i64::MAX).is_err());
    }
}
