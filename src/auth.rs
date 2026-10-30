use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::rngs::OsRng;
use rusqlite::Connection;

pub fn hash_password(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string()
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    let parsed = PasswordHash::new(hash).ok();
    if let Some(p) = parsed {
        Argon2::default()
            .verify_password(password.as_bytes(), &p)
            .is_ok()
    } else {
        false
    }
}

pub fn create_session(conn: &Connection, username: &str) -> String {
    let token = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp();
    let exp = now + 86400;
    conn.execute(
        "INSERT INTO sessions(token,username,created_at,expires_at) VALUES(?1,?2,?3,?4)",
        rusqlite::params![token, username, now, exp],
    )
    .ok();
    token
}

pub fn validate_session(conn: &Connection, token: &str) -> bool {
    let now = chrono::Utc::now().timestamp();
    conn.query_row(
        "SELECT 1 FROM sessions WHERE token=?1 AND expires_at>?2",
        rusqlite::params![token, now],
        |_| Ok(()),
    )
    .is_ok()
}

pub fn invalidate_all(conn: &Connection) {
    conn.execute("DELETE FROM sessions", []).ok();
}

pub fn has_credentials(conn: &Connection) -> bool {
    conn.query_row("SELECT 1 FROM credentials LIMIT 1", [], |_| Ok(()))
        .is_ok()
}

pub fn set_password(conn: &Connection, username: &str, password: &str) {
    let h = hash_password(password);
    conn.execute("DELETE FROM credentials", []).ok();
    conn.execute(
        "INSERT INTO credentials(username,password_hash) VALUES(?1,?2)",
        rusqlite::params![username, h],
    )
    .ok();
    invalidate_all(conn);
}

pub fn check_login(conn: &Connection, username: &str, password: &str) -> bool {
    let hash: Option<String> = conn
        .query_row(
            "SELECT password_hash FROM credentials WHERE username=?1",
            rusqlite::params![username],
            |r| r.get(0),
        )
        .ok();
    if let Some(h) = hash {
        verify_password(&h, password)
    } else {
        false
    }
}
