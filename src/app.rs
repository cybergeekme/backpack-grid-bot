use std::thread;

use anyhow::Result;
use chrono::Utc;

use crate::{
    backpack::{BackpackClient, SyncResult},
    config::Config,
    model::{CycleReport, OpenOrder, Position},
    persist, strategy,
    ws::{self, BackpackWsHandle, WsEvent},
};

pub struct App {
    config: Config,
    client: BackpackClient,
    ws: Option<BackpackWsHandle>,
    cached_mark_price: Option<rust_decimal::Decimal>,
    cached_position: Option<Position>,
    cached_orders: Option<Vec<OpenOrder>>,
    rest_cycle_counter: u64,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let client = BackpackClient::new(&config)?;
        let ws = ws::spawn(&config).ok();
        Ok(Self {
            config,
            client,
            ws,
            cached_mark_price: None,
            cached_position: None,
            cached_orders: None,
            rest_cycle_counter: 0,
        })
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
        self.ingest_ws_events()?;

        let do_rest_refresh = self.rest_cycle_counter == 0 || self.rest_cycle_counter % 4 == 0;
        let mark_price = if do_rest_refresh || self.cached_mark_price.is_none() {
            let value = self.client.fetch_mark_price(&self.config.symbol)?;
            self.cached_mark_price = Some(value);
            value
        } else {
            self.cached_mark_price.expect("mark price cached")
        };

        let position = if do_rest_refresh || self.cached_position.is_none() {
            let value = self.client.fetch_position(&self.config.symbol)?;
            self.cached_position = Some(value.clone());
            value
        } else {
            self.cached_position.clone().expect("position cached")
        };

        let existing = if do_rest_refresh || self.cached_orders.is_none() {
            let value = self.client.fetch_open_orders(&self.config.symbol)?;
            self.cached_orders = Some(value.clone());
            value
        } else {
            self.cached_orders.clone().expect("orders cached")
        };
        self.rest_cycle_counter = self.rest_cycle_counter.saturating_add(1);

        let (desired, paused, pause_reason, mut issues) = strategy::build_orders(&self.config, mark_price, &position);
        let sync = if paused {
            SyncResult {
                final_orders: existing.clone(),
                place_orders: Vec::new(),
                cancel_orders: Vec::new(),
                matched_orders: 0,
            }
        } else {
            self.client.sync_orders(&self.config.symbol, &desired, &existing)?
        };
        self.cached_orders = Some(sync.final_orders.clone());

        persist::info(&self.config.events_path, if self.config.live_enabled { "cycle executed in live mode" } else { "cycle executed in dry-run mode" })?;
        if self.ws.is_some() {
            persist::info(&self.config.events_path, "ws enabled with rest reconciliation fallback")?;
        } else {
            persist::info(&self.config.events_path, "ws unavailable, rest only mode")?;
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

    fn ingest_ws_events(&mut self) -> Result<()> {
        let Some(ws) = &self.ws else { return Ok(()); };
        for event in ws.poll_latest() {
            match event {
                WsEvent::Snapshot(snapshot) => {
                    if let Some(mark) = snapshot.mark_price {
                        self.cached_mark_price = Some(mark);
                    }
                    if let Some(position) = snapshot.position {
                        self.cached_position = Some(position);
                    }
                    if let Some(orders) = snapshot.open_orders {
                        self.cached_orders = Some(orders);
                    }
                }
                WsEvent::Info(msg) => persist::info(&self.config.events_path, &msg)?,
                WsEvent::Error(msg) => persist::info(&self.config.events_path, &msg)?,
            }
        }
        Ok(())
    }
}
