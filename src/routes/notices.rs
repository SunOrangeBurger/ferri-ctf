use std::sync::Arc;
use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::{errors::AppResult, middleware::auth::AuthUser, state::AppState};

/// Authenticated notices. Bait tokens remain team-scoped; system notices are global.
pub async fn team_notices(State(state): State<Arc<AppState>>, AuthUser(user): AuthUser) -> AppResult<Json<Value>> {
    let global: Vec<(String,String)> = sqlx::query_as(
        "SELECT id,message FROM system_notices
         WHERE expires_at IS NULL OR expires_at > strftime('%Y-%m-%d %H:%M:%f','now')
         ORDER BY created_at,id")
        .fetch_all(&state.db).await?;
    let mut notices: Vec<Value> = global.into_iter().map(|(id,message)| json!({
        "id":format!("system:{id}"), "message":message, "kind":"info"
    })).collect();
    if let Some(team) = user.team_id {
        let rows: Vec<(String,String,i64)> = sqlx::query_as(
            "SELECT p.challenge_id,c.title,a.delta FROM honeypot_pending p
             JOIN challenges c ON c.id=p.challenge_id
             JOIN score_adjustments a ON a.id='honeypot:'||p.team_id||':'||p.challenge_id
             WHERE p.team_id=? AND p.applied_at IS NOT NULL ORDER BY p.applied_at,p.challenge_id")
            .bind(&team).fetch_all(&state.db).await?;
        notices.extend(rows.into_iter().map(|(id,title,delta)| json!({
            "id":format!("{team}:{id}"),
            "message":format!("Instruction compliance: a token from ‘{title}’ was submitted despite instructions to skip it. A team score adjustment of {delta} points has been applied. Contact an organizer for review."),
            "kind":"penalty"
        })));
    }
    Ok(Json(json!({"notices":notices})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body,http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    #[tokio::test]
    async fn notices_only_expose_applied_penalties_to_the_same_team() {
        let (state,_dir) = crate::services::dynamic_scoring::tests::fixture(false,2).await;
        let id = crate::services::honeypots::IDS[0];
        sqlx::query("INSERT INTO challenges(id,title,description,category,difficulty,points,flag_hmac,flag_salt)
            VALUES (?,'Review','','Misc','easy',100,'x','x')").bind(id).execute(&state.db).await.unwrap();
        let mut tx = state.db.begin().await.unwrap();
        crate::services::honeypots::queue(&mut tx,"t0","u0",id,"2026-10-17 07:00:00.000").await.unwrap();
        tx.commit().await.unwrap();
        let app = crate::app::build(state.clone());
        async fn notices(app: &axum::Router,state: &AppState,user: &str) -> Value {
            let token = crate::session::create(&state.db,&state.keys,24,user).await.unwrap();
            let response = app.clone().oneshot(Request::builder().uri("/api/team/notices").header("host","localhost")
                .header("cookie",format!("{}={token}",crate::session::COOKIE_NAME)).body(Body::empty()).unwrap()).await.unwrap();
            let body = response.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice(&body).unwrap()
        }
        assert_eq!(notices(&app,&state,"u0").await["notices"].as_array().unwrap().len(),0);
        sqlx::query("UPDATE honeypot_pending SET due_at='2000-01-01 00:00:00'").execute(&state.db).await.unwrap();
        crate::services::honeypots::process_due(&state).await.unwrap();
        assert_eq!(notices(&app,&state,"u0").await["notices"].as_array().unwrap().len(),1);
        assert_eq!(notices(&app,&state,"u1").await["notices"].as_array().unwrap().len(),0);
    }
}
