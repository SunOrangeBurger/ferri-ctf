use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Challenge {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub difficulty: String,
    pub points: i64,
    pub flag_hmac: String,
    pub flag_salt: String,
    pub file_path: Option<String>,
    pub file_name: Option<String>,
    pub is_active: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChallengeCard {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub difficulty: String,
    pub points: i64,
    pub repeat_penalty: i64,
    pub has_file: bool,
    pub file_name: Option<String>,
    pub is_solved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TeamChallengePool {
    pub team_id: String,
    pub round_number: i64,
    pub challenge_id: String,
    pub repeat_penalty: i64,
    pub assigned_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Submission {
    pub id: String,
    pub team_id: String,
    pub user_id: String,
    pub challenge_id: String,
    pub is_correct: bool,
    pub submitted_hash: String,
    pub submitted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Solve {
    pub team_id: String,
    pub challenge_id: String,
    pub user_id: String,
    pub round_number: i64,
    pub points_awarded: i64,
    pub solved_at: String,
}
