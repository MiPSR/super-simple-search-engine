pub mod api;
pub mod auth;
mod canonical;
pub mod crawler;
pub mod db;
pub mod settings;
mod sitemap;
pub mod state;

pub fn db_init(path: &std::path::Path) -> rusqlite::Result<()> {
    db::init(path)
}
