use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Hint {
    pub id: String,
    pub challenge_id: String,
    pub content: String,
    pub point_cost: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct HintUsage {
    pub team_id: String,
    pub hint_id: String,
    pub used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HintView {
    pub id: String,
    pub challenge_id: String,
    pub point_cost: i64,
    pub is_unlocked: bool,
    pub content: Option<String>,
}
