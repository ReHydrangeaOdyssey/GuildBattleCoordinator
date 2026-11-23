# GuildBattleCoordinator

ReHydrangeaOdysseyのゲーム再現プロジェクトを構成するサーバーコンポーネントです。仕様の正本は[Specifications](https://github.com/ReHydrangeaOdyssey/Specifications)です。

## 実装済みの機能・基盤

- JSTの所定時間帯における決定的な騎士団マッチング。
- 所属ロック、0人候補の除外、候補0件時のエラー記録と終了処理。
- GameServer容量を比較した割当、Preload要求、scheduled対戦の再調整。
- Kubernetes EndpointSliceによるPod UIDの取得と、継続不在となった所有先の割当解除。

## 現在の制限

運営向けHTTP API、Scale-out Controller連携、Discord Bot通知は未接続です。GameServer側の騎士団戦runtime完成も必要です。

## 開発

リポジトリルートで次を実行します。未取得の依存がある場合、offline実行は失敗します。

```sh
cargo build --offline --locked
cargo clippy --offline --locked -- -D warnings
cargo fmt --check
```

起動設定は [src/config.rs](src/config.rs) を参照してください。設定JSONのパスを第1引数として渡します。証明書・秘密鍵・認証情報をリポジトリへ保存しないでください。

今後の実装作業向けskill: [develop-guild-battle-coordinator](.agents/skills/develop-guild-battle-coordinator/SKILL.md)。実装済みの範囲はソースと最新仕様を併せて確認してください。
