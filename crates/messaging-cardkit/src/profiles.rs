use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::sync::Arc;

use greentic_types::provider::ProviderDecl;

use crate::{CapabilityProfile, Tier};

/// Source of provider capability metadata used by operator runtimes.
pub trait ProfileSource: Send + Sync + Debug {
    /// Returns the tier associated with `provider_type`.
    fn tier(&self, provider_type: &str) -> Option<Tier>;

    /// Returns a capability profile (downgrade flags) for the provider.
    fn capability_profile(&self, provider_type: &str) -> Option<CapabilityProfile> {
        self.tier(provider_type).map(CapabilityProfile::for_tier)
    }

    /// Optional provider-specific button limit metadata (3, 5, etc.).
    fn button_limit(&self, _provider_type: &str) -> Option<usize> {
        None
    }
}

/// Simple `ProfileSource` backed by an explicit provider -> tier map.
#[derive(Debug, Default)]
pub struct StaticProfiles {
    tiers: HashMap<String, Tier>,
    buttons: HashMap<String, usize>,
    default_tier: Tier,
}

impl StaticProfiles {
    pub fn builder() -> StaticProfilesBuilder {
        StaticProfilesBuilder::default()
    }
}

impl ProfileSource for StaticProfiles {
    fn tier(&self, provider_type: &str) -> Option<Tier> {
        self.tiers
            .get(provider_type)
            .copied()
            .or(Some(self.default_tier))
    }

    fn button_limit(&self, provider_type: &str) -> Option<usize> {
        self.buttons.get(provider_type).copied()
    }
}

/// Builder for `StaticProfiles`.
#[derive(Default)]
pub struct StaticProfilesBuilder {
    tiers: HashMap<String, Tier>,
    buttons: HashMap<String, usize>,
    default_tier: Tier,
}

impl StaticProfilesBuilder {
    pub fn default_tier(mut self, tier: Tier) -> Self {
        self.default_tier = tier;
        self
    }

    pub fn for_provider(mut self, provider: impl Into<String>, tier: Tier) -> Self {
        self.tiers.insert(provider.into(), tier);
        self
    }

    pub fn button_limit(mut self, provider: impl Into<String>, limit: usize) -> Self {
        self.buttons.insert(provider.into(), limit);
        self
    }

    pub fn build(self) -> StaticProfiles {
        StaticProfiles {
            tiers: self.tiers,
            buttons: self.buttons,
            default_tier: self.default_tier,
        }
    }
}

/// Profile source derived from `ProviderDecl` metadata.
#[derive(Debug)]
pub struct PackProfiles {
    providers: HashMap<String, Arc<ProviderDecl>>,
}

impl PackProfiles {
    pub fn new<I>(decls: I) -> Self
    where
        I: IntoIterator<Item = ProviderDecl>,
    {
        let providers = decls
            .into_iter()
            .map(|decl| (decl.provider_type.clone(), Arc::new(decl)))
            .collect();
        Self { providers }
    }

    pub fn tier_from_caps(decl: &ProviderDecl) -> Tier {
        let caps = decl
            .capabilities
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        if caps.contains("supports_adaptive_cards") || caps.contains("premium") {
            Tier::Premium
        } else if caps.contains("advanced") || caps.contains("supports_factsets") {
            Tier::Advanced
        } else {
            Tier::Basic
        }
    }
}

impl ProfileSource for PackProfiles {
    fn tier(&self, provider_type: &str) -> Option<Tier> {
        self.providers
            .get(provider_type)
            .map(|decl| Self::tier_from_caps(decl))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::provider::ProviderRuntimeRef;

    fn stub_decl(provider_type: &str, caps: &[&str]) -> ProviderDecl {
        ProviderDecl {
            provider_type: provider_type.to_string(),
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            ops: Vec::new(),
            config_schema_ref: "config.schema.json".into(),
            state_schema_ref: None,
            runtime: ProviderRuntimeRef {
                component_ref: "component".into(),
                export: "run".into(),
                world: "messaging".into(),
            },
            docs_ref: None,
        }
    }

    #[test]
    fn pack_profiles_maps_capabilities_to_tier() {
        let profiles = PackProfiles::new(vec![
            stub_decl("premium", &["supports_adaptive_cards"]),
            stub_decl("advanced", &["supports_factsets"]),
            stub_decl("basic", &[]),
        ]);

        assert_eq!(profiles.tier("premium"), Some(Tier::Premium));
        assert_eq!(profiles.tier("advanced"), Some(Tier::Advanced));
        assert_eq!(profiles.tier("basic"), Some(Tier::Basic));
        assert_eq!(profiles.tier("unknown"), None);
    }

    #[test]
    fn tier_from_caps_chooses_highest_tier() {
        let premium_caps = ["premium", "supports_factsets"];
        let advanced_caps = ["supports_factsets"];
        let basic_caps: [&str; 0] = [];

        assert_eq!(
            PackProfiles::tier_from_caps(&stub_decl("premium", &premium_caps)),
            Tier::Premium
        );
        assert_eq!(
            PackProfiles::tier_from_caps(&stub_decl("advanced", &advanced_caps)),
            Tier::Advanced
        );
        assert_eq!(
            PackProfiles::tier_from_caps(&stub_decl("basic", &basic_caps)),
            Tier::Basic
        );
    }
}
