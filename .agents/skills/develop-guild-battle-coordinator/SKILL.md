---
name: develop-guild-battle-coordinator
description: GuildBattleCoordinatorのマッチング・割当・EndpointSlice監視を変更する際に使用する。
---

# GuildBattleCoordinatorの変更

以下のパスはリポジトリルート基準。隣接する `../Specifications` の最新仕様と実装を確認してから変更する。参照先がない場合は仕様の所在を確認し、READMEやこのskillを仕様の代用にしない。

## 参照と実装境界

Specifications/design/server/ 配下の guild_battle_coordinator.md、guild_battle_lifecycle.md と design/system/api_transport.md・api_request_validation.md を対象機能に応じて読む。通信型を変更するときはSpecificationsのprotoと各利用側のコピー・生成設定を照合する。

- `matching.rs` のPCG・seed・並べ替え・Battle ID生成は決定的であることが必要。時刻単位とJSTの日付を区別する。
- `coordinator.rs` のマッチングでは所属ロック後に候補を再取得し、DB上の既存対戦を優先する。0人除外一覧の保存と終了時の解除を維持する。
- 容量比較は空きthread数、使用率、LastAssignedAt、Instance IDの仕様順。割当直後の予約をローカル容量に反映し、Preload応答の受付集合を確認する。
- `discovery.rs` のInstance IDはEndpointSliceのPod UID。Discovery失敗を全Pod消失と扱わない。不在継続による割当解除はscheduledだけに適用する。
- 再起動時もDBのscheduledを再調整する。in_progress/resolvingを再割当しない。
- Scale-out ControllerとBotの通知契約は未接続。CoordinatorからDeploymentのreplicasを直接変更する機能を追加しない。

## 検証と引継ぎ

既存テストが対象機能を検証する場合は実行し、`cargo build --offline --locked`、`cargo clippy --offline --locked -- -D warnings`、`cargo fmt --check` で確認する。依存未取得なら無断で外部取得せず、必要なものを示す。新規テストの作成は適用されるAGENTS.mdとユーザー指示に従う。

READMEの実装済み機能と未接続箇所を実コードに合わせる。運用環境が必要な未実施検証をビルド結果と区別して報告する。skill自体は依存追加、DB migration実行、pushやデプロイへの許可を与えない。
