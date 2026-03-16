use pi_lynx_sdk::{EmbedErrorKind, ProviderSelection, ProviderStreamOverride, resolve_provider};
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

/// WHY: host config may use Pi aliases and conservative stream overrides, so
/// embed resolution must normalize them before runtime assembly starts.
#[test]
fn resolve_provider_normalizes_aliases_and_applies_overrides() {
    let mut headers = BTreeMap::new();
    headers.insert("x-host".to_string(), "lynx".to_string());

    let resolved = resolve_provider(&ProviderSelection {
        provider_id: "open-router".to_string(),
        model_id: "auto".to_string(),
        api_key: Some(" host-key ".to_string()),
        thinking: Some(pi::model::ThinkingLevel::XHigh),
        stream_options_override: Some(ProviderStreamOverride {
            temperature: Some(0.5),
            max_tokens: Some(2048),
            headers,
            reasoning_budget_tokens: Some(4096),
        }),
    })
    .expect("provider resolves");

    assert_eq!(resolved.provider_id(), "openrouter");
    assert_eq!(resolved.model_id(), "openrouter/auto");
    assert_eq!(resolved.provider.name(), "openrouter");
    assert_eq!(resolved.provider.model_id(), "openrouter/auto");
    assert_eq!(resolved.stream_options.api_key.as_deref(), Some("host-key"));
    assert_eq!(resolved.stream_options.temperature, Some(0.5_f32));
    assert_eq!(resolved.stream_options.max_tokens, Some(2048));
    assert_eq!(
        resolved
            .stream_options
            .headers
            .get("x-host")
            .map(String::as_str),
        Some("lynx")
    );
    assert_eq!(
        resolved.stream_options.thinking_level,
        Some(pi::model::ThinkingLevel::High)
    );
    assert_eq!(
        resolved
            .stream_options
            .thinking_budgets
            .as_ref()
            .map(|budgets| budgets.high),
        Some(4096)
    );
}

/// WHY: unsupported providers need to fail before runtime assembly so Lynx can
/// surface a deterministic configuration error instead of a late provider crash.
#[test]
fn resolve_provider_rejects_unknown_embed_provider_defaults() {
    let error = match resolve_provider(&ProviderSelection {
        provider_id: "definitely-unknown".to_string(),
        model_id: "model-x".to_string(),
        api_key: None,
        thinking: None,
        stream_options_override: None,
    }) {
        Ok(_) => panic!("unknown provider should fail"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), EmbedErrorKind::InvalidConfig);
    assert!(
        error
            .to_string()
            .contains("does not expose embed-safe routing defaults")
    );
}
