//! WebChat GUI messaging provider component.

mod bindings {
    wit_bindgen::generate!({
        path: "wit/messaging-provider-webchat-gui",
        world: "component-v0-v6-v0",
        generate_all
    });
}

pub(crate) mod config;
#[allow(dead_code)]
#[path = "../../messaging-provider-webchat/src/describe.rs"]
mod describe;
pub(crate) mod directline;
mod embed_origins;
mod gui_core;
#[path = "../../messaging-provider-webchat/src/ops/mod.rs"]
mod ops;

pub(crate) const PROVIDER_ID: &str = "messaging-provider-webchat-gui";
pub(crate) const PROVIDER_TYPE: &str = "messaging.webchat-gui";
pub(crate) const WORLD_ID: &str = "component-v0-v6-v0";
pub(crate) const DEFAULT_SKIN: &str = "default";
pub(crate) const DEFAULT_OAUTH_ENABLED: bool = false;

use gui_core::Component;

bindings::export!(Component with_types_in bindings);
