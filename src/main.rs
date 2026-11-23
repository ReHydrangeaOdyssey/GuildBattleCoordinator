#[tokio::main]
async fn main() -> std::process::ExitCode {
    match guild_battle_coordinator::config::run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("GuildBattleCoordinator startup or server failure");
            std::process::ExitCode::FAILURE
        }
    }
}
