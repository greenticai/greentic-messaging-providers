use anyhow::Result;
use provider_tests::harness::{self, instantiate};

fn instantiate_component(name: &str) -> Result<()> {
    let engine = harness::new_engine();
    instantiate::instantiate_provider(&engine, &harness::component_path(name))
}

#[test]
fn instantiates_slack() -> Result<()> {
    instantiate_component("messaging-provider-slack")
}

#[test]
fn instantiates_telegram() -> Result<()> {
    instantiate_component("messaging-provider-telegram")
}

#[test]
fn instantiates_teams() -> Result<()> {
    instantiate_component("messaging-provider-teams")
}

#[test]
fn instantiates_whatsapp() -> Result<()> {
    instantiate_component("messaging-provider-whatsapp")
}

#[test]
fn instantiates_webex() -> Result<()> {
    instantiate_component("messaging-provider-webex")
}

#[test]
fn instantiates_email() -> Result<()> {
    instantiate_component("messaging-provider-email")
}

#[test]
fn instantiates_webchat() -> Result<()> {
    instantiate_component("messaging-provider-webchat")
}

#[test]
fn instantiates_dummy() -> Result<()> {
    instantiate_component("messaging-provider-dummy")
}
