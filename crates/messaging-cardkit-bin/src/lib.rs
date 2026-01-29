use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use clap::{Args, Parser, Subcommand};
use messaging_cardkit::{
    CapabilityProfile, CardKit, PlatformPreview, RenderIntent, RenderResponse, StaticProfiles, Tier,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(author, version, about = "CardKit render demo CLI/server")]
pub struct Cli {
    #[command(flatten)]
    profile: ProfileArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
pub struct ProfileArgs {
    /// Default tier for providers that are not explicitly mapped.
    #[arg(long, default_value = "basic")]
    default_tier: TierArg,

    /// Override provider tiers (format: provider=tier).
    #[arg(long = "provider-tier", value_parser = ProviderTierArg::from_str)]
    provider_tiers: Vec<ProviderTierArg>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Render a MessageCard fixture via CardKit.
    Render(RenderOptions),
    /// Serve an HTTP render/demo endpoint backed by CardKit.
    Serve(ServeOptions),
}

#[derive(Args)]
pub struct RenderOptions {
    /// Provider type to render.
    #[arg(long)]
    provider: String,

    /// Path to a MessageCard JSON fixture.
    #[arg(long)]
    fixture: PathBuf,
}

#[derive(Args)]
pub struct ServeOptions {
    /// Host to bind the server to.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Port to listen on.
    #[arg(long, default_value_t = 7878)]
    port: u16,

    /// Directory containing MessageCard JSON fixtures referenced by /render.
    #[arg(long, default_value = "tests/fixtures/cards")]
    fixtures_dir: PathBuf,
}

#[derive(Clone)]
struct ProfileConfig {
    default_tier: Tier,
    provider_overrides: Vec<ProviderOverride>,
}

#[derive(Clone)]
struct ProviderOverride {
    provider: String,
    tier: Tier,
}

impl ProfileArgs {
    fn into_config(self) -> ProfileConfig {
        ProfileConfig {
            default_tier: self.default_tier.tier,
            provider_overrides: self
                .provider_tiers
                .into_iter()
                .map(|arg| ProviderOverride {
                    provider: arg.provider,
                    tier: arg.tier,
                })
                .collect(),
        }
    }
}

impl ProfileConfig {
    fn build_static_profiles(&self) -> StaticProfiles {
        let mut builder = StaticProfiles::builder().default_tier(self.default_tier);
        for provider_override in &self.provider_overrides {
            builder =
                builder.for_provider(provider_override.provider.clone(), provider_override.tier);
        }
        builder.build()
    }

    fn providers_info(&self) -> Vec<ProviderInfo> {
        let mut providers = self
            .provider_overrides
            .iter()
            .map(|provider_override| ProviderInfo {
                provider: provider_override.provider.clone(),
                tier: provider_override.tier,
            })
            .collect::<Vec<_>>();
        providers.push(ProviderInfo {
            provider: "*default".to_string(),
            tier: self.default_tier,
        });
        providers
    }
}

#[derive(Clone)]
struct AppState {
    kit: Arc<CardKit<StaticProfiles>>,
    providers: Vec<ProviderInfo>,
    fixtures_dir: PathBuf,
}

#[derive(Serialize, Clone)]
struct ProviderInfo {
    provider: String,
    tier: Tier,
}

#[derive(Deserialize)]
struct RenderRequest {
    provider: String,
    fixture: Option<String>,
    card: Option<Value>,
}

#[derive(Serialize)]
struct RenderResponseDto {
    intent: RenderIntent,
    payload: Value,
    preview: PlatformPreviewDto,
    warnings: Vec<String>,
    capability: Option<CapabilityProfileDto>,
}

#[derive(Serialize)]
struct PlatformPreviewDto {
    payload: Value,
    tier: Tier,
    target_tier: Tier,
    downgraded: bool,
    used_modal: bool,
    limit_exceeded: bool,
    sanitized_count: usize,
    url_blocked_count: usize,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct CapabilityProfileDto {
    allow_images: bool,
    allow_factset: bool,
    allow_inputs: bool,
    allow_postbacks: bool,
}

impl From<RenderResponse> for RenderResponseDto {
    fn from(response: RenderResponse) -> Self {
        RenderResponseDto {
            intent: response.intent,
            payload: response.payload,
            preview: PlatformPreviewDto::from(response.preview),
            warnings: response.warnings,
            capability: response.capability.map(CapabilityProfileDto::from),
        }
    }
}

impl From<PlatformPreview> for PlatformPreviewDto {
    fn from(preview: PlatformPreview) -> Self {
        PlatformPreviewDto {
            payload: preview.payload,
            tier: preview.tier,
            target_tier: preview.target_tier,
            downgraded: preview.downgraded,
            used_modal: preview.used_modal,
            limit_exceeded: preview.limit_exceeded,
            sanitized_count: preview.sanitized_count,
            url_blocked_count: preview.url_blocked_count,
            warnings: preview.warnings,
        }
    }
}

impl From<CapabilityProfile> for CapabilityProfileDto {
    fn from(profile: CapabilityProfile) -> Self {
        CapabilityProfileDto {
            allow_images: profile.allow_images,
            allow_factset: profile.allow_factset,
            allow_inputs: profile.allow_inputs,
            allow_postbacks: profile.allow_postbacks,
        }
    }
}

fn build_cardkit(config: &ProfileConfig) -> Arc<CardKit<StaticProfiles>> {
    let profiles = config.build_static_profiles();
    Arc::new(CardKit::new(Arc::new(profiles)))
}

fn load_fixture(path: &Path) -> Result<Value> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read fixture {}", path.display()))?;
    serde_json::from_str(&data)
        .with_context(|| format!("fixture {} is not valid JSON", path.display()))
}

async fn run_serve(opts: ServeOptions, config: ProfileConfig) -> Result<()> {
    let kit = build_cardkit(&config);
    let state = AppState {
        kit,
        providers: config.providers_info(),
        fixtures_dir: opts.fixtures_dir,
    };

    let addr: SocketAddr = format!("{}:{}", opts.host, opts.port)
        .parse()
        .context("invalid host/port")?;

    let app = Router::new()
        .route("/render", post(render_handler))
        .route("/providers", get(providers_handler))
        .with_state(state);

    println!("listening on http://{}", addr);
    let listener = TcpListener::bind(addr).await.context("bind failed")?;
    axum::serve(listener, app).await.context("server failed")
}

fn run_render(opts: RenderOptions, config: &ProfileConfig) -> Result<()> {
    let kit = build_cardkit(config);
    let card = load_fixture(&opts.fixture)?;
    let response = kit.render(&opts.provider, &card).context("render failed")?;
    let dto = RenderResponseDto::from(response);
    println!("{}", serde_json::to_string_pretty(&dto)?);
    Ok(())
}

async fn render_handler(
    State(state): State<AppState>,
    Json(payload): Json<RenderRequest>,
) -> Result<Json<RenderResponseDto>, (StatusCode, String)> {
    let card = load_request_card(&payload, &state.fixtures_dir)
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let response = state
        .kit
        .render(&payload.provider, &card)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(RenderResponseDto::from(response)))
}

fn load_request_card(request: &RenderRequest, fixtures_dir: &Path) -> Result<Value, String> {
    if let Some(card) = request.card.clone() {
        return Ok(card);
    }
    if let Some(fixture) = &request.fixture {
        let path = fixtures_dir.join(fixture);
        let data = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read fixture {}: {err}", path.display()))?;
        let value = serde_json::from_str(&data)
            .map_err(|err| format!("fixture {} is not valid JSON: {err}", path.display()))?;
        return Ok(value);
    }
    Err("either card or fixture must be provided".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cards")
    }

    #[test]
    fn load_request_card_prefers_inline_card() {
        let request = RenderRequest {
            provider: "slack".to_string(),
            fixture: Some("basic.json".to_string()),
            card: Some(json!({ "kind": "inline" })),
        };
        let card = load_request_card(&request, fixtures_dir().as_path()).expect("inline card");
        assert_eq!(card.get("kind"), Some(&json!("inline")));
    }

    #[test]
    fn load_request_card_reads_fixture_when_no_inline() {
        let request = RenderRequest {
            provider: "slack".to_string(),
            fixture: Some("basic.json".to_string()),
            card: None,
        };
        let card = load_request_card(&request, fixtures_dir().as_path()).expect("fixture card");
        assert_eq!(card.get("type"), Some(&json!("AdaptiveCard")));
    }

    #[test]
    fn load_request_card_errors_without_payload() {
        let request = RenderRequest {
            provider: "slack".to_string(),
            fixture: None,
            card: None,
        };
        let err = load_request_card(&request, fixtures_dir().as_path()).unwrap_err();
        assert!(err.contains("either card or fixture must be provided"));
    }
}

async fn providers_handler(State(state): State<AppState>) -> Json<ProvidersResponse> {
    Json(ProvidersResponse {
        providers: state.providers.clone(),
    })
}

#[derive(Serialize)]
struct ProvidersResponse {
    providers: Vec<ProviderInfo>,
}

#[derive(Debug, Clone)]
struct TierArg {
    tier: Tier,
}

impl FromStr for TierArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "basic" => Ok(TierArg { tier: Tier::Basic }),
            "advanced" => Ok(TierArg {
                tier: Tier::Advanced,
            }),
            "premium" => Ok(TierArg {
                tier: Tier::Premium,
            }),
            other => Err(format!(
                "unknown tier '{other}', expected basic/advanced/premium"
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct ProviderTierArg {
    provider: String,
    tier: Tier,
}

impl FromStr for ProviderTierArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (provider, tier) = value
            .split_once('=')
            .ok_or_else(|| "provider-tier must use format provider=tier".to_string())?;
        let tier = TierArg::from_str(tier)?.tier;
        Ok(ProviderTierArg {
            provider: provider.to_string(),
            tier,
        })
    }
}

pub async fn run(cli: Cli) -> Result<()> {
    let config = cli.profile.into_config();
    match cli.command {
        Command::Render(opts) => run_render(opts, &config),
        Command::Serve(opts) => run_serve(opts, config).await,
    }
}
