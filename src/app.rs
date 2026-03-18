use std::thread;

use anyhow::Result;
use chrono::Utc;

use crate::{
    backpack::BackpackClient,
    config::Config,
    model::CycleReport,
    persist,
    strategy,
};

pub struct App {
    config: Config,
    client: BackpackClient,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let client = BackpackClient::new(&config)?;
        Ok(Self { config, client })
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            self.run_cycle()?;
            if self.config.run_once {
                return Ok(());
            }
            thread::sleep(self.config.loop_interval);
        }
    }

    fn run_cycle(&mut self) -> Result<()> {
        let mark_price = self.client.fetch_mark_price(&self.config.symbol)?;
        let position = self.client.fetch_position(&self.config.symbol)?;
        let existing = self.client.fetch_open_orders(&self.config.symbol)?;
        let (desired, paused, pause_reason, mut issues) = strategy::build_orders(&self.config, mark_price, &position);
        let sync = if paused {
            crate::backpack::SyncResult {
                final_orders: existing.clone(),
                place_orders: Vec::new(),
                cancel_orders: Vec::new(),
                matched_orders: 0,
            }
        } else {
            self.client.sync_orders(&self.config.symbol, &desired, &existing)?
        };

        if self.config.live_enabled {
            persist::info(&self.config.events_path, "cycle executed in live mode")?;
        } else {
            persist::info(&self.config.events_path, "cycle executed in dry-run mode")?;
        }
        if paused {
            persist::info(&self.config.events_path, pause_reason.as_deref().unwrap_or("paused"))?;
        }
        if position.size.abs() > self.config.max_position_abs {
            issues.push("position_already_over_limit".to_string());
        }

        let report = CycleReport {
            ts: Utc::now().to_rfc3339(),
            symbol: self.config.symbol.clone(),
            mode: if self.config.live_enabled { "live".to_string() } else { "dry_run".to_string() },
            mark_price,
            position_size: position.size,
            entry_price: position.entry_price,
            desired_orders: desired.len(),
            existing_orders: existing.len(),
            place_orders: sync.place_orders.len(),
            cancel_orders: sync.cancel_orders.len(),
            matched_orders: sync.matched_orders,
            paused,
            pause_reason,
            issues,
        };

        persist::save_state(&self.config.state_path, report.clone())?;
        persist::append_jsonl(&self.config.cycles_path, &report)?;

        println!(
            "{} symbol={} mode={} mark={} pos={} desired={} existing={} place={} cancel={} matched={} paused={}",
            report.ts,
            report.symbol,
            report.mode,
            report.mark_price,
            report.position_size,
            report.desired_orders,
            report.existing_orders,
            report.place_orders,
            report.cancel_orders,
            report.matched_orders,
            report.paused,
        );
        Ok(())
    }
}
