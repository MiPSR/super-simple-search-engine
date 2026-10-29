pub mod api;
pub mod auth;
pub mod crawler;
pub mod db;
pub mod settings;
pub mod state;

pub fn db_init(path: &std::path::Path) -> rusqlite::Result<()> {
    db::init(path)
}
