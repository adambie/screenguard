use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, patch, post},
    Json, Router,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use std::sync::Arc;

use crate::state::AppState;

pub mod auth;
pub mod agents;
pub mod apps;
pub mod profiles;
pub mod usage;
pub mod users;

pub fn router(state: Arc<AppState>) -> Router {
    let public = Router::new()
        .route("/version", get(auth::version))
        .route("/auth/status", get(auth::status))
        .route("/auth/setup", post(auth::setup))
        .route("/auth/login", post(auth::login));

    let protected = Router::new()
        // Auth (me)
        .route("/auth/me", get(auth::get_me).patch(auth::patch_me))
        .route("/auth/password", post(auth::change_password))
        // User management (any admin)
        .route("/users", get(users::list_users).post(users::create_user))
        .route("/users/{id}", delete(users::delete_user))
        .route("/users/{id}/reset-password", post(users::reset_password))
        // Agents
        .route("/agents", get(agents::list_agents))
        .route("/agents/{id}", get(agents::get_agent).patch(agents::patch_agent).delete(agents::delete_agent))
        .route("/agents/{id}/accept", post(agents::accept_agent))
        .route("/agents/{id}/undo-delete", post(agents::undo_delete_agent))
        .route("/agents/{id}/force-delete", post(agents::force_delete_agent))
        .route("/agents/{id}/users", get(agents::list_agent_users))
        .route("/agents/{id}/logs", get(agents::fetch_agent_logs))
        .route("/agents/{id}/update", post(agents::update_agent))
        // Agent users
        .route("/agent-users/{id}", patch(profiles::patch_agent_user))
        // Profiles
        .route("/profiles", get(profiles::list_profiles).post(profiles::create_profile))
        .route("/profiles/{id}", get(profiles::get_profile).patch(profiles::patch_profile).delete(profiles::delete_profile))
        // Schedules
        .route("/profiles/{id}/schedules", get(profiles::get_schedules).put(profiles::replace_schedules))
        // Limits
        .route("/profiles/{id}/daily-limits", get(profiles::get_daily_limits).put(profiles::replace_daily_limits))
        // Adjustments
        .route("/profiles/{id}/adjustments", get(profiles::list_adjustments).post(profiles::create_adjustment))
        .route("/profiles/{id}/lock-now", post(profiles::lock_now))
        .route("/profiles/{id}/notify", post(profiles::notify_profile))
        // Blocked domains
        .route("/profiles/{id}/blocked-domains", get(profiles::get_blocked_domains).put(profiles::set_blocked_domains).post(profiles::add_blocked_domain))
        .route("/profiles/{id}/blocked-domains/{domain_id}", patch(profiles::patch_blocked_domain).delete(profiles::delete_blocked_domain))
        // Apps and app groups
        .route("/profiles/{id}/apps", get(apps::board))
        .route("/profiles/{id}/app-groups", post(apps::create_group))
        .route("/profiles/{id}/app-groups/{gid}", patch(apps::rename_group).delete(apps::delete_group))
        .route("/profiles/{id}/app-groups/{gid}/rules", post(apps::add_rule))
        .route("/profiles/{id}/app-groups/{gid}/rules/{rule_id}", delete(apps::delete_rule))
        .route("/profiles/{id}/app-groups/{gid}/membership", post(apps::set_membership))
        .route("/profiles/{id}/app-group-limits/{gid}", axum::routing::put(apps::put_group_limits))
        .route("/profiles/{id}/app-group-adjustments", post(apps::add_group_adjustment))
        .route("/profiles/{id}/app-approvals", get(apps::get_approvals))
        .route("/profiles/{id}/app-approvals/mode", axum::routing::put(apps::set_approval_mode))
        .route("/profiles/{id}/app-approvals/decision", post(apps::decide_approval))
        .route("/app-requests", get(apps::pending_requests))
        // Usage & dashboard
        .route("/profiles/{id}/usage", get(usage::get_usage))
        .route("/profiles/{id}/app-usage", get(usage::get_app_usage))
        .route("/profiles/{id}/status", get(usage::get_status))
        .route("/dashboard", get(usage::dashboard))
        .layer(middleware::from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .nest("/api/v1", public.merge(protected))
        .with_state(state)
}

async fn require_auth(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut req: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Missing token" })))
        })?;

    let claims = decode::<auth::Claims>(
        token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?
    .claims;

    // Re-check the account on every request: a deleted admin's still-valid token
    // must stop working immediately, and an admin whose password was chosen by
    // someone else may do nothing but set their own.
    let admin_id: uuid::Uuid = claims.sub.parse()
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;
    let admin = crate::db::get_admin_user_by_id(&state.db, admin_id)
        .await
        .map_err(auth::internal)?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid token" }))))?;
    let path = req.uri().path();
    let allowed_before_change = path.ends_with("/auth/password")
        || (path.ends_with("/auth/me") && req.method() == axum::http::Method::GET);
    if admin.must_change_password && !allowed_before_change {
        return Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Password change required",
                "code": "password_change_required",
            })),
        ));
    }

    req.extensions_mut().insert(claims);
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request as HttpRequest};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    async fn app() -> Router {
        app_with_state().await.0
    }

    async fn app_with_state() -> (Router, Arc<AppState>) {
        let pool = crate::db::open_in_memory().await;
        let state = AppState::new(pool, "test-secret".into(), 1);
        (router(state.clone()), state)
    }

    async fn admin_token(app: &Router) -> String {
        call(app, Method::POST, "/auth/setup", None, Some(json!({ "username": "mom", "password": "pw" }))).await;
        login(app, "mom", "pw").await.1["token"].as_str().unwrap().to_string()
    }

    /// Simulates an agent connected and waiting, as ws::handle_pairing_request leaves it.
    async fn pending_agent(state: &Arc<AppState>, machine_id: &str, code: &str)
        -> (uuid::Uuid, tokio::sync::oneshot::Receiver<crate::state::PairingDecision>)
    {
        let agent = crate::db::upsert_agent_pending(&state.db, machine_id, "kid-pc", "UTC", "").await.unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.pending.write().await.insert(
            (crate::state::DEFAULT_TENANT.to_string(), machine_id.to_string()),
            crate::state::PairingHandle { pairing_code: code.into(), tx, conn_id: uuid::Uuid::new_v4(), failed_attempts: 0 },
        );
        (agent.id, rx)
    }

    #[tokio::test]
    async fn accept_requires_the_agents_pairing_code() {
        let (app, state) = app_with_state().await;
        let tok = admin_token(&app).await;
        let (id, rx) = pending_agent(&state, "m1", "ABCDEF").await;
        let accept = |code: &'static str| {
            let (app, tok) = (app.clone(), tok.clone());
            async move {
                call(&app, Method::POST, &format!("/agents/{id}/accept"), Some(&tok), Some(json!({ "pairing_code": code }))).await
            }
        };

        // The code is never exposed by the API.
        let (_, list) = call(&app, Method::GET, "/agents", Some(&tok), None).await;
        assert!(!list.to_string().contains("ABCDEF"));
        assert_eq!(list["agents"][0]["pairing_requested"], true);
        let (_, detail) = call(&app, Method::GET, &format!("/agents/{id}"), Some(&tok), None).await;
        assert!(!detail.to_string().contains("ABCDEF"));

        let (s, body) = accept("ZZZZZZ").await;
        assert_eq!((s, body["code"].as_str(), body["attempts_left"].as_u64()), (StatusCode::FORBIDDEN, Some("wrong_code"), Some(4)));
        assert_eq!(crate::db::get_agent_by_id(&state.db, id).await.unwrap().unwrap().status, "pending");

        // Separators and case don't matter.
        let (s, _) = accept("abc-def").await;
        assert_eq!(s, StatusCode::OK);
        let decision = rx.await.unwrap();
        assert_eq!(decision.agent_db_id, id);
        let agent = crate::db::get_agent_by_id(&state.db, id).await.unwrap().unwrap();
        assert_eq!(agent.status, "paired");
        assert_eq!(agent.auth_token_hash.as_deref(), Some(agents::hash_token(&decision.auth_token).as_str()));

        // One-time: a second accept finds nothing waiting and changes nothing.
        let (s, body) = accept("ABCDEF").await;
        assert_eq!((s, body["code"].as_str()), (StatusCode::CONFLICT, Some("not_waiting")));
        assert_eq!(crate::db::get_agent_by_id(&state.db, id).await.unwrap().unwrap().auth_token_hash, agent.auth_token_hash);
    }

    #[tokio::test]
    async fn too_many_wrong_codes_drop_the_request() {
        let (app, state) = app_with_state().await;
        let tok = admin_token(&app).await;
        let (id, rx) = pending_agent(&state, "m1", "ABCDEF").await;
        for left in (0..5).rev() {
            let (s, body) = call(&app, Method::POST, &format!("/agents/{id}/accept"), Some(&tok),
                Some(json!({ "pairing_code": "WRONG1" }))).await;
            assert_eq!((s, body["attempts_left"].as_u64()), (StatusCode::FORBIDDEN, Some(left)));
        }
        assert!(rx.await.is_err(), "dropping the handle ends the agent's pairing connection");
        let (s, body) = call(&app, Method::POST, &format!("/agents/{id}/accept"), Some(&tok),
            Some(json!({ "pairing_code": "ABCDEF" }))).await;
        assert_eq!((s, body["code"].as_str()), (StatusCode::CONFLICT, Some("not_waiting")));
        assert_eq!(crate::db::get_agent_by_id(&state.db, id).await.unwrap().unwrap().status, "pending");
    }

    #[tokio::test]
    async fn pairing_request_does_not_unpair_a_paired_agent() {
        let (app, state) = app_with_state().await;
        let tok = admin_token(&app).await;
        let (id, rx) = pending_agent(&state, "m1", "ABCDEF").await;
        call(&app, Method::POST, &format!("/agents/{id}/accept"), Some(&tok), Some(json!({ "pairing_code": "ABCDEF" }))).await;
        let token_hash = crate::db::get_agent_by_id(&state.db, id).await.unwrap().unwrap().auth_token_hash;
        drop(rx);

        // Someone sends a pairing request with the same (world-readable) machine-id.
        let _rx2 = pending_agent(&state, "m1", "EVIL22").await;
        let agent = crate::db::get_agent_by_id(&state.db, id).await.unwrap().unwrap();
        assert_eq!(agent.status, "paired");
        assert_eq!(agent.auth_token_hash, token_hash);
    }

    #[tokio::test]
    async fn app_usage_report() {
        let (app, state) = app_with_state().await;
        let tok = admin_token(&app).await;
        let (a1, _, _, profile) = crate::app_usage::tests::seeded(&state.db).await;
        crate::db::update_agent_app_usage(&state.db, a1, true).await.unwrap();
        let get = |q: String| {
            let (app, tok) = (app.clone(), tok.clone());
            async move { call(&app, Method::GET, &format!("/profiles/{profile}/app-usage{q}"), Some(&tok), None).await }
        };

        let (s, day) = get("?range=day&date=2026-10-20".into()).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(day["total_seconds"], 6600);
        assert_eq!(day["apps"][0]["app_id"], "steam:570");
        assert_eq!(day["apps"][0]["name"], "Dota 2");
        assert_eq!(day["apps"][1]["seconds"], 3000);
        assert_eq!(day["app_usage_available"], true);
        assert_eq!(day["days"].as_array().unwrap().len(), 0);

        // 2026-10-22 is a Thursday: the week is Mon 19 – Sun 25
        let (_, week) = get("?range=week&date=2026-10-22".into()).await;
        assert_eq!((week["from"].as_str(), week["to"].as_str()), (Some("2026-10-19"), Some("2026-10-25")));
        assert_eq!(week["total_seconds"], 7200);
        let days = week["days"].as_array().unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!((days[0]["seconds"].as_i64(), days[1]["seconds"].as_i64(), days[2]["seconds"].as_i64()), (Some(600), Some(6600), Some(0)));

        let (_, total) = get("?range=total".into()).await;
        assert_eq!(total["total_seconds"], 7260);

        // by group: Dota 2 (steam:) is in Games; with a Tuesday limit of 90 min
        let games = crate::app_groups::list_groups(&state.db, profile).await.unwrap()[0].id.clone();
        crate::app_groups::replace_limits(&state.db, &profile.to_string(), &games, &[(1, 90)]).await.unwrap();
        let (_, day) = get("?range=day&date=2026-10-20".into()).await; // a Tuesday
        let g = day["groups"].as_array().unwrap().iter().find(|g| g["id"] == games.as_str()).unwrap();
        assert_eq!((g["seconds"].as_i64(), g["limit_minutes"].as_i64()), (Some(3600), Some(90)));
        let (_, week) = get("?range=week&date=2026-10-22".into()).await;
        let g = week["groups"].as_array().unwrap().iter().find(|g| g["id"] == games.as_str()).unwrap();
        assert_eq!((g["seconds"].as_i64(), g["limit_minutes"].is_null()), (Some(4200), true));
        // once the agent reports group time, that wins over the sum of apps
        let sync = common::messages::AppUsageSync {
            entries: vec![],
            groups: vec![common::messages::GroupUsageEntry { local_uid: 1001, date: "2026-10-20".parse().unwrap(), group_id: games.clone(), seconds_total: 3000 }],
        };
        crate::app_usage::ingest(&state.db, a1, &sync, "2026-10-20".parse().unwrap()).await.unwrap();
        let (_, day) = get("?range=day&date=2026-10-20".into()).await;
        let g = day["groups"].as_array().unwrap().iter().find(|g| g["id"] == games.as_str()).unwrap();
        assert_eq!(g["seconds"].as_i64(), Some(3000));

        let (s, _) = get("?range=year".into()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = get("?date=20-10-2026".into()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = call(&app, Method::GET, &format!("/profiles/{}/app-usage", uuid::Uuid::new_v4()), Some(&tok), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = call(&app, Method::GET, &format!("/profiles/{profile}/app-usage"), None, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn apps_and_groups_api() {
        let (app, state) = app_with_state().await;
        let tok = admin_token(&app).await;
        let (_, _, _, pid) = crate::app_usage::tests::seeded(&state.db).await; // firefox, steam:570 (Game), foot
        let c = |m: Method, p: String, b: Option<Value>| {
            let (app, tok) = (app.clone(), tok.clone());
            async move { call(&app, m, &p, Some(&tok), b).await }
        };
        let base = format!("/profiles/{pid}");

        // the child's board: their apps (with today's time) and their groups
        let (s, board) = c(Method::GET, format!("{base}/apps"), None).await;
        assert_eq!(s, StatusCode::OK);
        let groups = board["groups"].as_array().unwrap();
        let (games, browsers) = (groups[0]["id"].as_str().unwrap().to_string(), groups[1]["id"].as_str().unwrap().to_string());
        assert_eq!(groups[0]["builtin"], "games");
        let app_of = |b: &Value, id: &str| b["apps"].as_array().unwrap().iter().find(|a| a["app_id"] == id).unwrap().clone();
        assert_eq!(app_of(&board, "steam:570")["group"], games.as_str());
        assert!(app_of(&board, "desktop:firefox")["group"].is_null());

        // put Firefox into "Web browsers"; take Dota out of Games
        let (s, _) = c(Method::POST, format!("{base}/app-groups/{browsers}/membership"), Some(json!({"app_id": "desktop:firefox", "member": true}))).await;
        assert_eq!(s, StatusCode::OK);
        c(Method::POST, format!("{base}/app-groups/{games}/membership"), Some(json!({"app_id": "steam:570", "member": false}))).await;
        let (_, board) = c(Method::GET, format!("{base}/apps"), None).await;
        assert_eq!(app_of(&board, "desktop:firefox")["group"], browsers.as_str());
        assert!(app_of(&board, "steam:570")["group"].is_null());

        // custom group, rule, rename, delete
        let (s, created) = c(Method::POST, format!("{base}/app-groups"), Some(json!({"name": "Education"}))).await;
        assert_eq!(s, StatusCode::CREATED);
        let id = created["id"].as_str().unwrap().to_string();
        let (s, _) = c(Method::POST, format!("{base}/app-groups/{id}/rules"), Some(json!({"kind": "category", "value": "Education"}))).await;
        assert_eq!(s, StatusCode::CREATED);
        let (s, _) = c(Method::POST, format!("{base}/app-groups/{id}/rules"), Some(json!({"kind": "category", "value": "Bad;"}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = c(Method::PATCH, format!("{base}/app-groups/{id}"), Some(json!({"name": "School"}))).await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = c(Method::DELETE, format!("{base}/app-groups/{id}"), None).await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = c(Method::DELETE, format!("{base}/app-groups/{id}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);

        // limits and extra time show on the board
        let (s, _) = c(Method::PUT, format!("{base}/app-group-limits/{games}"),
            Some(json!({"limits": [{"day_of_week": 0, "allowed_minutes": 60}, {"day_of_week": 6, "allowed_minutes": 0}]}))).await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = c(Method::PUT, format!("{base}/app-group-limits/{games}"), Some(json!({"limits": [{"day_of_week": 9, "allowed_minutes": 60}]}))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = c(Method::POST, format!("{base}/app-group-adjustments"), Some(json!({"group_id": games, "minutes": 15}))).await;
        assert_eq!(s, StatusCode::CREATED);
        let (_, board) = c(Method::GET, format!("{base}/apps"), None).await;
        assert_eq!(board["groups"][0]["limits"], json!([{"day_of_week": 0, "allowed_minutes": 60}, {"day_of_week": 6, "allowed_minutes": 0}]));
        assert_eq!(board["groups"][0]["adjustment_today"], 15);

        // another child's group is out of reach from this profile
        let (_, other) = c(Method::POST, "/profiles".into(), Some(json!({"display_name": "Other"}))).await;
        let oid = other["id"].as_str().or(other["profile"]["id"].as_str()).unwrap().to_string();
        let (s, _) = c(Method::DELETE, format!("/profiles/{oid}/app-groups/{games}"), None).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) = c(Method::PUT, format!("/profiles/{oid}/app-group-limits/{games}"), Some(json!({"limits": []}))).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, oboard) = c(Method::GET, format!("/profiles/{oid}/apps"), None).await;
        assert_eq!(oboard["groups"].as_array().unwrap().len(), 2); // its own predefined groups
        assert!(oboard["apps"].as_array().unwrap().is_empty()); // and none of the first child's apps

        // new-app approval: mode, a request, decision into one of the child's groups
        let (s, _) = c(Method::PUT, format!("{base}/app-approvals/mode"), Some(json!({"enabled": true}))).await;
        assert_eq!(s, StatusCode::OK);
        crate::app_approvals::request(&state.db, pid, "steam:1091500").await.unwrap();
        let (_, r) = c(Method::GET, "/app-requests".into(), None).await;
        assert_eq!(r["requests"][0]["app_id"], "steam:1091500");
        let (s, _) = c(Method::POST, format!("{base}/app-approvals/decision"),
            Some(json!({"app_id": "steam:1091500", "approve": true, "group_id": browsers}))).await;
        assert_eq!(s, StatusCode::OK);
        let (_, r) = c(Method::GET, "/app-requests".into(), None).await;
        assert!(r["requests"].as_array().unwrap().is_empty());
        let (s, _) = call(&app, Method::GET, &format!("{base}/apps"), None, None).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }

    async fn call(app: &Router, method: Method, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
        let mut req = HttpRequest::builder().method(method).uri(format!("/api/v1{path}"));
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let req = match body {
            Some(b) => req.header("content-type", "application/json").body(Body::from(b.to_string())),
            None => req.body(Body::empty()),
        }
        .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn login(app: &Router, username: &str, password: &str) -> (StatusCode, Value) {
        call(app, Method::POST, "/auth/login", None, Some(json!({ "username": username, "password": password }))).await
    }

    #[tokio::test]
    async fn multi_admin_flow() {
        let app = app().await;
        let (s, _) = call(&app, Method::POST, "/auth/setup", None,
            Some(json!({ "username": "mom", "password": "pw1", "timezone": "Europe/Warsaw" }))).await;
        assert_eq!(s, StatusCode::CREATED);
        let (_, body) = login(&app, "mom", "pw1").await;
        assert_eq!(body["must_change_password"], false);
        let mom = body["token"].as_str().unwrap().to_string();

        // Any admin can add another; the new account inherits the installation timezone.
        let (s, created) = call(&app, Method::POST, "/users", Some(&mom),
            Some(json!({ "username": " dad ", "password": "temp" }))).await;
        assert_eq!(s, StatusCode::CREATED);
        assert_eq!(created["username"], "dad");
        let dad_id = created["id"].as_str().unwrap().to_string();

        // First login with the temporary password: everything but GET /auth/me
        // and the password change is blocked.
        let (_, body) = login(&app, "dad", "temp").await;
        assert_eq!(body["must_change_password"], true);
        let dad = body["token"].as_str().unwrap().to_string();
        let (s, me) = call(&app, Method::GET, "/auth/me", Some(&dad), None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(me["timezone"], "Europe/Warsaw");
        let (s, body) = call(&app, Method::GET, "/profiles", Some(&dad), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "password_change_required");
        let (s, _) = call(&app, Method::PATCH, "/auth/me", Some(&dad), Some(json!({ "timezone": "UTC" }))).await;
        assert_eq!(s, StatusCode::FORBIDDEN);

        // Setting their own password (proving the temporary one) unlocks the account.
        let (s, _) = call(&app, Method::POST, "/auth/password", Some(&dad),
            Some(json!({ "current_password": "temp", "new_password": "dadpw" }))).await;
        assert_eq!(s, StatusCode::OK);
        let (s, _) = call(&app, Method::GET, "/profiles", Some(&dad), None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(login(&app, "dad", "temp").await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(login(&app, "dad", "dadpw").await.1["must_change_password"], false);

        // A later change requires the current password.
        let (s, body) = call(&app, Method::POST, "/auth/password", Some(&dad),
            Some(json!({ "current_password": "wrong", "new_password": "x" }))).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert_eq!(body["code"], "wrong_password");

        // Timezone changed by either admin applies installation-wide.
        let (s, _) = call(&app, Method::PATCH, "/auth/me", Some(&dad), Some(json!({ "timezone": "Europe/Berlin" }))).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(call(&app, Method::GET, "/auth/me", Some(&mom), None).await.1["timezone"], "Europe/Berlin");

        // No self-delete / self-reset.
        let (s, body) = call(&app, Method::DELETE, &format!("/users/{dad_id}"), Some(&dad), None).await;
        assert_eq!((s, body["code"].as_str()), (StatusCode::CONFLICT, Some("self_delete")));
        let (s, body) = call(&app, Method::POST, &format!("/users/{dad_id}/reset-password"), Some(&dad),
            Some(json!({ "password": "p" }))).await;
        assert_eq!((s, body["code"].as_str()), (StatusCode::CONFLICT, Some("self_reset")));

        // A reset by another admin re-arms the forced change.
        let (s, _) = call(&app, Method::POST, &format!("/users/{dad_id}/reset-password"), Some(&mom),
            Some(json!({ "password": "temp2" }))).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(call(&app, Method::GET, "/profiles", Some(&dad), None).await.0, StatusCode::FORBIDDEN);
        // A session that was already open can't take over the account: it
        // doesn't know the new temporary password.
        let (s, body) = call(&app, Method::POST, "/auth/password", Some(&dad),
            Some(json!({ "current_password": "dadpw", "new_password": "mine" }))).await;
        assert_eq!((s, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("wrong_password")));

        // Deleting an admin revokes their still-unexpired token immediately.
        let (s, _) = call(&app, Method::DELETE, &format!("/users/{dad_id}"), Some(&mom), None).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(call(&app, Method::GET, "/auth/me", Some(&dad), None).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(call(&app, Method::GET, "/users", Some(&mom), None).await.1["users"].as_array().unwrap().len(), 1);
    }
}
