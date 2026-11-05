//! 3AIgent GUI messaging provider component.
//!
//! A WebChat GUI variant: same Direct Line backend and SPA, shipped with the
//! 3AIgent skin and OAuth login enabled by default.

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-3aigent-gui",
        world: "component-v0-v6-v0",
        generate_all
    });
}

#[path = "../../messaging-provider-webchat-gui/src/config.rs"]
pub(crate) mod config;
#[allow(dead_code)]
#[path = "../../messaging-provider-webchat/src/describe.rs"]
mod describe;
#[path = "../../messaging-provider-webchat-gui/src/directline/mod.rs"]
pub(crate) mod directline;
#[path = "../../messaging-provider-webchat-gui/src/embed_origins.rs"]
mod embed_origins;
#[path = "../../messaging-provider-webchat-gui/src/gui_core.rs"]
mod gui_core;
#[path = "../../messaging-provider-webchat/src/ops/mod.rs"]
mod ops;

pub(crate) const PROVIDER_ID: &str = "messaging-provider-3aigent-gui";
pub(crate) const PROVIDER_TYPE: &str = "messaging.3aigent-gui";
pub(crate) const WORLD_ID: &str = "component-v0-v6-v0";
pub(crate) const DEFAULT_SKIN: &str = "3aigent";
pub(crate) const DEFAULT_OAUTH_ENABLED: bool = true;

use gui_core::Component;

bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod variant_tests {
    #[test]
    fn variant_identity_differs_from_webchat_gui() {
        assert_eq!(super::PROVIDER_TYPE, "messaging.3aigent-gui");
        assert_eq!(super::PROVIDER_ID, "messaging-provider-3aigent-gui");
        assert_eq!(super::DEFAULT_SKIN, "3aigent");
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(super::DEFAULT_OAUTH_ENABLED);
        }
    }

    #[test]
    fn tenant_template_carries_skin_and_all_oauth_providers() {
        const TEMPLATE: &str = include_str!(
            "../../../packs/messaging-3aigent-gui/assets/webchat-gui/config/tenants/default.json"
        );
        let cfg: serde_json::Value = serde_json::from_str(TEMPLATE).unwrap();

        assert_eq!(cfg["skin"], "3aigent");

        let providers = cfg["auth"]["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 5);
        assert_eq!(
            providers[0]["id"], "greentic",
            "greentic must be listed first"
        );

        let ids: Vec<&str> = providers
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        for expected in ["greentic", "google", "microsoft", "github", "custom"] {
            assert!(ids.contains(&expected), "missing provider id {expected}");
        }
        // greentic leads the list as the default *choice*; enablement is owned by
        // the runtime /auth/config gate, so the template ships every provider off.
        for provider in providers {
            assert_eq!(
                provider["enabled"], false,
                "provider {} must ship disabled",
                provider["id"]
            );
        }
    }
}
