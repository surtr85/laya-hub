pub mod config;
pub mod mcp;
pub mod scorer;
pub mod supervisor;

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "laya-hub",
    about = "Laya System-1 MCP & Skill Supervisor and Dynamic Router"
)]
struct Args {
    #[arg(short, long, default_value = "~/.config/laya-hub/config.yaml")]
    config: PathBuf,

    /// Test route mode: scores a task and prints matched capabilities to stdout then exits
    #[arg(short, long)]
    test_route: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let config_path = shellexpand::tilde(&args.config.to_string_lossy()).to_string();

    let app_config = Arc::new(config::AppConfig::load_from_file(&config_path)?);

    let scorer = Arc::new(scorer::LayaScorer::new(
        app_config.laya.url.clone(),
        app_config.laya.threshold,
        app_config.laya.max_active_tools,
    ));

    // CLI quick test route mode
    if let Some(task) = args.test_route {
        let decision = scorer.route_task(&task, &app_config).await?;
        println!("{}", serde_json::to_string_pretty(&decision)?);
        return Ok(());
    }

    let supervisor = Arc::new(supervisor::ProcessSupervisor::new(app_config.clone()));
    supervisor
        .clone()
        .start_idle_sweeper(app_config.laya.idle_timeout_secs);

    let handler = mcp::McpHandler::new(app_config, scorer, supervisor);
    handler.run_stdio_loop().await?;

    Ok(())
}
