use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub is_ferris: bool,
    pub is_banned: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UserPublic {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub is_admin: bool,
    pub is_ferris: bool,
    pub is_banned: bool,
    pub created_at: String,
}
