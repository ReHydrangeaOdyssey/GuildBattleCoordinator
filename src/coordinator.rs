use crate::{
    protocol::game::privateapi::*,
    rpc::{self, Failure},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
#[derive(Clone, serde::Deserialize)]
pub struct Endpoint {
    pub instance_id: String,
    pub url: String,
    pub ready: bool,
}
pub struct Coordinator {
    pub client: reqwest::Client,
    pub private_url: String,
    missing_since: BTreeMap<String, Instant>,
    last_assigned: BTreeMap<String, u64>,
    pub missing_grace: Duration,
}
struct Candidate {
    endpoint: Endpoint,
    capacity: GetGameServerCapacityResponse,
    last: u64,
}
impl Coordinator {
    pub fn new(client: reqwest::Client, private_url: String) -> Self {
        Self {
            client,
            private_url,
            missing_since: BTreeMap::new(),
            last_assigned: BTreeMap::new(),
            missing_grace: Duration::from_secs(15),
        }
    }
    async fn private<Q: prost::Message, R: prost::Message + Default>(
        &self,
        method: &str,
        request: Q,
    ) -> Result<R, Failure> {
        rpc::call(
            &self.client,
            &self.private_url,
            &format!("/v1/private/{method}"),
            request,
            None,
            None,
        )
        .await
    }
    async fn preload(
        &self,
        endpoint: &Endpoint,
        battles: Vec<ScheduledGuildBattle>,
    ) -> Result<(), Failure> {
        let expected: BTreeSet<_> = battles.iter().map(|b| b.guild_battle_id).collect();
        let response: StartGuildBattlePreloadResponse = rpc::call(
            &self.client,
            &endpoint.url,
            "/v1/control/StartGuildBattlePreload",
            StartGuildBattlePreloadRequest { battles },
            None,
            None,
        )
        .await?;
        let accepted: BTreeSet<_> = response.accepted_guild_battle_id.into_iter().collect();
        if accepted != expected {
            return Err(Failure::UNAVAILABLE);
        }
        Ok(())
    }
    pub async fn reconcile(
        &mut self,
        endpoints: &[Endpoint],
        now: u64,
    ) -> Result<Vec<u64>, Failure> {
        let state: GetGuildBattleCoordinationStateResponse = self
            .private(
                "GetGuildBattleCoordinationState",
                GetGuildBattleCoordinationStateRequest {},
            )
            .await?;
        let mut pending = Vec::new();
        let mut blocked = BTreeSet::new();
        let present: BTreeSet<_> = endpoints.iter().map(|e| e.instance_id.clone()).collect();
        self.missing_since
            .retain(|instance, _| !present.contains(instance));
        for entry in state.battles {
            if entry.status != 0 {
                return Err(Failure::UNAVAILABLE);
            }
            let battle = entry.battle.ok_or(Failure::UNAVAILABLE)?;
            if !entry.has_assignment {
                pending.push(battle.guild_battle_id);
                continue;
            }
            if let Some(endpoint) = endpoints
                .iter()
                .find(|e| e.instance_id == entry.game_server_instance_id)
            {
                if !endpoint.ready || self.preload(endpoint, vec![battle]).await.is_err() {
                    blocked.insert(endpoint.instance_id.clone());
                }
            } else {
                let since = self
                    .missing_since
                    .entry(entry.game_server_instance_id.clone())
                    .or_insert_with(Instant::now);
                if since.elapsed() < self.missing_grace {
                    continue;
                }
                let response: ReleaseScheduledGuildBattleAssignmentsResponse = self
                    .private(
                        "ReleaseScheduledGuildBattleAssignments",
                        ReleaseScheduledGuildBattleAssignmentsRequest {
                            game_server_instance_id: entry.game_server_instance_id,
                            guild_battle_id: vec![battle.guild_battle_id],
                        },
                    )
                    .await?;
                pending.extend(response.released_guild_battle_id);
            }
        }
        let mut candidates = self.capacities(endpoints, &blocked).await;
        let mut unassigned = Vec::new();
        // Preserve the DB's StartAt/ID order. Never resend a previously observed capacity after a failed preload.
        for id in pending {
            candidates.sort_by(compare);
            let Some(candidate) = candidates.first_mut() else {
                unassigned.push(id);
                continue;
            };
            let response: AssignScheduledGuildBattlesResponse = self
                .private(
                    "AssignScheduledGuildBattles",
                    AssignScheduledGuildBattlesRequest {
                        game_server_instance_id: candidate.endpoint.instance_id.clone(),
                        guild_battle_id: vec![id],
                    },
                )
                .await?;
            if response.battles.is_empty() {
                continue;
            }
            if response.battles.len() != 1 || response.battles[0].guild_battle_id != id {
                return Err(Failure::UNAVAILABLE);
            }
            self.last_assigned
                .insert(candidate.endpoint.instance_id.clone(), now);
            candidate.last = now;
            candidate.capacity.available_guild_battle_count = candidate
                .capacity
                .available_guild_battle_count
                .saturating_sub(1);
            // A just-assigned battle may occupy an empty thread; refresh capacity only after preload acknowledgement.
            if self
                .preload(&candidate.endpoint, response.battles)
                .await
                .is_err()
            {
                candidates.remove(0);
            } else {
                let endpoint = candidate.endpoint.clone();
                let refreshed = self.capacities(&[endpoint], &BTreeSet::new()).await;
                candidates.remove(0);
                candidates.extend(refreshed);
            }
        }
        Ok(unassigned)
    }
    async fn capacities(
        &self,
        endpoints: &[Endpoint],
        blocked: &BTreeSet<String>,
    ) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        for endpoint in endpoints
            .iter()
            .filter(|e| e.ready && !blocked.contains(&e.instance_id))
        {
            let capacity: Result<GetGameServerCapacityResponse, _> = rpc::call(
                &self.client,
                &endpoint.url,
                "/v1/control/GetGameServerCapacity",
                GetGameServerCapacityRequest {},
                None,
                None,
            )
            .await;
            if let Ok(capacity) = capacity {
                if capacity.game_server_instance_id != endpoint.instance_id
                    || !capacity.accept_new_guild_battle
                    || capacity.available_guild_battle_count == 0
                    || capacity.total_guild_battle_thread_count == 0
                    || capacity.available_guild_battle_thread_count
                        > capacity.total_guild_battle_thread_count
                {
                    continue;
                }
                candidates.push(Candidate {
                    endpoint: endpoint.clone(),
                    capacity,
                    last: self
                        .last_assigned
                        .get(&endpoint.instance_id)
                        .copied()
                        .unwrap_or(0),
                });
            }
        }
        candidates
    }
    pub async fn match_slot(
        &self,
        target_date: u64,
        slot: i32,
        ymd: u64,
        now: u64,
    ) -> Result<(), Failure> {
        let initial: GetGuildsForBattleMatchingResponse = self
            .private(
                "GetGuildsForBattleMatching",
                GetGuildsForBattleMatchingRequest {
                    target_date,
                    start_time: Some(slot),
                },
            )
            .await?;
        let _: SetGuildMembershipLockResponse = self
            .private(
                "SetGuildMembershipLock",
                SetGuildMembershipLockRequest {
                    guild_id: initial.candidates.into_iter().map(|c| c.guild_id).collect(),
                    locked: Some(true),
                },
            )
            .await?;
        let current: GetGuildsForBattleMatchingResponse = self
            .private(
                "GetGuildsForBattleMatching",
                GetGuildsForBattleMatchingRequest {
                    target_date,
                    start_time: Some(slot),
                },
            )
            .await?;
        let existing: GetScheduledGuildsResponse = self
            .private(
                "GetScheduledGuilds",
                GetScheduledGuildsRequest {
                    target_date,
                    start_time: Some(slot),
                },
            )
            .await?;
        if !existing.battles.is_empty() {
            return Ok(());
        }
        let mut excluded: Vec<_> = current
            .candidates
            .iter()
            .filter(|c| c.member_count == 0)
            .map(|c| c.guild_id)
            .collect();
        excluded.sort_unstable();
        let _: SaveGuildBattleExcludedGuildsResponse = self
            .private(
                "SaveGuildBattleExcludedGuilds",
                SaveGuildBattleExcludedGuildsRequest {
                    target_date,
                    start_time: Some(slot),
                    guild_id: excluded.clone(),
                },
            )
            .await?;
        let guilds: Vec<_> = current
            .candidates
            .into_iter()
            .filter(|c| c.member_count != 0)
            .map(|c| c.guild_id)
            .collect();
        if guilds.is_empty() {
            return self
                .finish_empty_slot(target_date, slot, now, excluded)
                .await;
        }
        let battles = crate::matching::pairs(guilds, ymd, slot, now)?;
        let _: SaveScheduledGuildBattlesResponse = self
            .private(
                "SaveScheduledGuildBattles",
                SaveScheduledGuildBattlesRequest {
                    target_date,
                    start_time: Some(slot),
                    battles,
                },
            )
            .await?;
        Ok(())
    }
    async fn finish_empty_slot(
        &self,
        target_date: u64,
        slot: i32,
        now: u64,
        excluded: Vec<u64>,
    ) -> Result<(), Failure> {
        let _: SetGuildMembershipLockResponse = self
            .private(
                "SetGuildMembershipLock",
                SetGuildMembershipLockRequest {
                    guild_id: excluded,
                    locked: Some(false),
                },
            )
            .await?;
        let _: SaveErrorLogResponse = self
            .private(
                "SaveErrorLog",
                SaveErrorLogRequest {
                    time: now,
                    guild_battle_id: 0,
                    error_log: format!(
                        "No populated guilds for matching; target_date={target_date}; slot={slot}"
                    ),
                },
            )
            .await?;
        let _: ClearGuildBattleExcludedGuildsResponse = self
            .private(
                "ClearGuildBattleExcludedGuilds",
                ClearGuildBattleExcludedGuildsRequest {
                    target_date,
                    start_time: Some(slot),
                },
            )
            .await?;
        Ok(())
    }
}
fn compare(a: &Candidate, b: &Candidate) -> std::cmp::Ordering {
    let a_cap = &a.capacity;
    let b_cap = &b.capacity;
    // Compare occupied/total without float rounding: a.occupied * b.total versus b.occupied * a.total.
    let a_use = u64::from(
        a_cap.total_guild_battle_thread_count - a_cap.available_guild_battle_thread_count,
    ) * u64::from(b_cap.total_guild_battle_thread_count);
    let b_use = u64::from(
        b_cap.total_guild_battle_thread_count - b_cap.available_guild_battle_thread_count,
    ) * u64::from(a_cap.total_guild_battle_thread_count);
    b_cap
        .available_guild_battle_thread_count
        .cmp(&a_cap.available_guild_battle_thread_count)
        .then(a_use.cmp(&b_use))
        .then(a.last.cmp(&b.last))
        .then(a.endpoint.instance_id.cmp(&b.endpoint.instance_id))
}
