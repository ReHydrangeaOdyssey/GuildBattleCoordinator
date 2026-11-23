use crate::{
    coordinator::{Coordinator, Endpoint},
    transport::Error,
};
use std::{collections::BTreeSet, time::Duration};
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    internal_ca: String,
    internal_identity: String,
    private_url: String,
    endpoints: Vec<Endpoint>,
    kubernetes: Option<crate::discovery::Kubernetes>,
    reconcile_seconds: u64,
}
pub async fn run() -> Result<(), Error> {
    let path = std::env::args()
        .nth(1)
        .ok_or("configuration JSON path required")?;
    let config: Config = serde_json::from_slice(&tokio::fs::read(path).await?)?;
    if config.reconcile_seconds == 0 || config.reconcile_seconds > 30 {
        return Err("reconcile interval must be 1..30 seconds".into());
    }
    if config.kubernetes.is_some() && !config.endpoints.is_empty() {
        return Err("choose Kubernetes or development endpoints".into());
    }
    let client =
        crate::transport::internal_client(&config.internal_ca, &config.internal_identity).await?;
    let mut coordinator = Coordinator::new(client, config.private_url);
    let mut completed = BTreeSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(config.reconcile_seconds));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            signal=tokio::signal::ctrl_c()=>{ signal?; return Ok(()); }
            _=interval.tick()=>{}
        }
        let now = time::OffsetDateTime::now_utc().to_offset(time::UtcOffset::from_hms(9, 0, 0)?);
        let micros = u64::try_from(now.unix_timestamp_nanos() / 1000)?;
        // YYYYMMDD = year * 10000 + month * 100 + day.
        let ymd = now.year() as u64 * 10_000
            + u64::from(u8::from(now.month())) * 100
            + u64::from(now.day());
        completed.retain(|(date, _)| *date == ymd);
        for (slot, (hour, minute)) in [(11, 30), (12, 15), (13, 0), (21, 0), (22, 0), (23, 0)]
            .into_iter()
            .enumerate()
        {
            let start = now
                .date()
                .with_hms(hour, minute, 0)?
                .assume_offset(now.offset());
            let remaining = (start - now).whole_seconds();
            if (0..=300).contains(&remaining) && !completed.contains(&(ymd, slot)) {
                match coordinator
                    .match_slot(micros, slot as i32, ymd, micros)
                    .await
                {
                    Ok(()) => {
                        completed.insert((ymd, slot));
                    }
                    Err(_) => {
                        eprintln!("GuildBattle matching failed; slot={slot}");
                    }
                }
            }
        }
        let endpoints = match &config.kubernetes {
            Some(kubernetes) => match kubernetes.endpoints().await {
                Ok(endpoints) => endpoints,
                // A discovery outage is not proof that every owner disappeared.
                Err(_) => {
                    eprintln!("EndpointSlice discovery failed");
                    continue;
                }
            },
            None => config.endpoints.clone(),
        };
        match coordinator.reconcile(&endpoints, micros).await {
            Ok(unassigned) if !unassigned.is_empty() => {
                eprintln!(
                    "GuildBattle capacity unavailable; unassigned={}",
                    unassigned.len()
                );
            }
            Ok(_) => {}
            Err(_) => {
                eprintln!("GuildBattle reconciliation failed");
            }
        }
    }
}
