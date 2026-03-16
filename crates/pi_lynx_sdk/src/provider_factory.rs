//! Provider/model resolution for Lynx embedding.

use crate::errors::{EmbedError, Result};
use crate::types::ProviderSelection;
use pi::models::ModelEntry;
use pi::provider::ThinkingBudgets;
use pi::provider_metadata::{
    ProviderRoutingDefaults, canonical_provider_id, provider_routing_defaults,
};
use pi::providers::create_provider;
use pi::sdk::{InputType, Model, ModelCost, Provider, StreamOptions};
use std::collections::HashMap;
use std::sync::Arc;

const OPENAI_CODEX_RESPONSES_BASE_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const GOOGLE_GEMINI_CLI_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";
const GOOGLE_ANTIGRAVITY_BASE_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";

const INPUT_TEXT_IMAGE: [InputType; 2] = [InputType::Text, InputType::Image];

/// Resolved provider runtime inputs assembled from host selection.
pub struct ResolvedProvider {
    /// Normalized Pi model entry used to create the provider.
    pub model_entry: ModelEntry,
    /// Provider instance selected for the host request.
    pub provider: Arc<dyn Provider>,
    /// Conservative stream options derived from the host request.
    pub stream_options: StreamOptions,
}

impl ResolvedProvider {
    /// Return the normalized provider identifier.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.model_entry.model.provider
    }

    /// Return the normalized model identifier.
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_entry.model.id
    }
}

/// Resolve a host provider selection into a Pi provider, model entry, and stream options.
pub fn resolve_provider(selection: &ProviderSelection) -> Result<ResolvedProvider> {
    let model_entry = resolve_model_entry(selection)?;
    let stream_options = build_stream_options(selection, &model_entry)?;
    let provider = create_provider(&model_entry, None).map_err(|source| {
        EmbedError::bootstrap(
            "provider_factory::create_provider",
            Some(model_entry.model.provider.clone()),
            Some(model_entry.model.id.clone()),
            source,
        )
    })?;

    tracing::debug!(
        provider = %model_entry.model.provider,
        model_id = %model_entry.model.id,
        api = %model_entry.model.api,
        "Resolved Lynx embed provider"
    );

    Ok(ResolvedProvider {
        model_entry,
        provider,
        stream_options,
    })
}

/// Normalize host selection into a Pi [`ModelEntry`].
pub fn resolve_model_entry(selection: &ProviderSelection) -> Result<ModelEntry> {
    let raw_provider_id = selection.provider_id.trim();
    if raw_provider_id.is_empty() {
        return Err(EmbedError::config(
            "provider_factory::resolve_model_entry",
            "provider_id must not be empty",
        ));
    }

    let raw_model_id = selection.model_id.trim();
    if raw_model_id.is_empty() {
        return Err(EmbedError::config(
            "provider_factory::resolve_model_entry",
            "model_id must not be empty",
        ));
    }

    let provider_id = canonical_provider_id(raw_provider_id).unwrap_or(raw_provider_id);
    let model_id = canonicalize_model_id(provider_id, raw_model_id);
    let defaults = provider_defaults(provider_id).ok_or_else(|| {
        EmbedError::config(
            "provider_factory::resolve_model_entry",
            format!("provider '{provider_id}' does not expose embed-safe routing defaults"),
        )
    })?;

    Ok(ModelEntry {
        model: Model {
            id: model_id.clone(),
            name: model_id,
            api: defaults.api.to_string(),
            provider: provider_id.to_string(),
            base_url: defaults.base_url.to_string(),
            reasoning: defaults.reasoning,
            input: defaults.input.to_vec(),
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: defaults.context_window,
            max_tokens: defaults.max_tokens,
            headers: HashMap::new(),
        },
        api_key: normalize_optional_string(selection.api_key.as_deref()),
        headers: HashMap::new(),
        auth_header: defaults.auth_header,
        compat: None,
        oauth_config: None,
    })
}

/// Build conservative stream options from host selection and the normalized model entry.
pub fn build_stream_options(
    selection: &ProviderSelection,
    entry: &ModelEntry,
) -> Result<StreamOptions> {
    let mut stream_options = StreamOptions {
        api_key: normalize_optional_string(selection.api_key.as_deref()),
        headers: HashMap::new(),
        thinking_level: selection
            .thinking
            .map(|level| entry.clamp_thinking_level(level)),
        ..StreamOptions::default()
    };

    if let Some(overrides) = selection.stream_options_override.as_ref() {
        if let Some(temperature) = overrides.temperature {
            if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
                return Err(EmbedError::config(
                    "provider_factory::build_stream_options",
                    "temperature must be finite and within 0.0..=2.0",
                ));
            }
            stream_options.temperature = Some(temperature);
        }

        if let Some(max_tokens) = overrides.max_tokens {
            if max_tokens == 0 {
                return Err(EmbedError::config(
                    "provider_factory::build_stream_options",
                    "max_tokens must be greater than zero",
                ));
            }
            stream_options.max_tokens = Some(max_tokens);
        }

        if let Some(reasoning_budget_tokens) = overrides.reasoning_budget_tokens {
            if reasoning_budget_tokens == 0 {
                return Err(EmbedError::config(
                    "provider_factory::build_stream_options",
                    "reasoning_budget_tokens must be greater than zero",
                ));
            }
            stream_options.thinking_budgets = Some(ThinkingBudgets {
                minimal: reasoning_budget_tokens,
                low: reasoning_budget_tokens,
                medium: reasoning_budget_tokens,
                high: reasoning_budget_tokens,
                xhigh: reasoning_budget_tokens,
            });
        }

        for (key, value) in &overrides.headers {
            let key = key.trim();
            if key.is_empty() {
                return Err(EmbedError::config(
                    "provider_factory::build_stream_options",
                    "stream override headers must not contain blank names",
                ));
            }
            stream_options
                .headers
                .insert(key.to_string(), value.trim().to_string());
        }
    }

    Ok(stream_options)
}

fn provider_defaults(provider_id: &str) -> Option<ProviderRoutingDefaults> {
    provider_routing_defaults(provider_id).or(match provider_id {
        "openai-codex" => Some(ProviderRoutingDefaults {
            api: "openai-codex-responses",
            base_url: OPENAI_CODEX_RESPONSES_BASE_URL,
            auth_header: true,
            reasoning: true,
            input: &INPUT_TEXT_IMAGE,
            context_window: 128_000,
            max_tokens: 16_384,
        }),
        "google-gemini-cli" => Some(ProviderRoutingDefaults {
            api: "google-gemini-cli",
            base_url: GOOGLE_GEMINI_CLI_BASE_URL,
            auth_header: false,
            reasoning: true,
            input: &INPUT_TEXT_IMAGE,
            context_window: 128_000,
            max_tokens: 8_192,
        }),
        "google-antigravity" => Some(ProviderRoutingDefaults {
            api: "google-gemini-cli",
            base_url: GOOGLE_ANTIGRAVITY_BASE_URL,
            auth_header: false,
            reasoning: true,
            input: &INPUT_TEXT_IMAGE,
            context_window: 128_000,
            max_tokens: 8_192,
        }),
        "sap-ai-core" | "sap" => None,
        _ => None,
    })
}

fn canonicalize_model_id(provider_id: &str, model_id: &str) -> String {
    if provider_id.eq_ignore_ascii_case("openrouter") {
        return match model_id.trim().to_ascii_lowercase().as_str() {
            "auto" => "openrouter/auto".to_string(),
            "gpt-4o-mini" => "openai/gpt-4o-mini".to_string(),
            "gpt-4o" => "openai/gpt-4o".to_string(),
            "claude-3.5-sonnet" => "anthropic/claude-3.5-sonnet".to_string(),
            "gemini-2.5-pro" => "google/gemini-2.5-pro".to_string(),
            _ => model_id.trim().to_string(),
        };
    }

    model_id.trim().to_string()
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}
