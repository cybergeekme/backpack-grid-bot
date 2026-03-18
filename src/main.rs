mod app;
mod backpack;
mod config;
mod model;
mod persist;
mod strategy;
mod ws;

use anyhow::Result;

fn main() -> Result<()> {
    let config = config::Config::from_env()?;
    let mut app = app::App::new(config)?;
    app.run()
}
