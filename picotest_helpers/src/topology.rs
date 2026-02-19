use anyhow::Context;
use pike::cluster::Tier;
use std::{fs, path::PathBuf};

use crate::migration::MigrationContextProvider;

pub const DEFAULT_TIER: &str = "default";

pub type PluginTopology = pike::cluster::Topology;

pub fn parse_topology(path: &PathBuf) -> anyhow::Result<PluginTopology> {
    toml::from_str(
        &fs::read_to_string(path).context(format!("Failed to read file '{}'", path.display()))?,
    )
    .context(format!(
        "Failed to parse topology TOML from path '{}'",
        path.display()
    ))
}

pub trait TopologyTransformer {
    fn transform(&self, source_topology: &PluginTopology) -> PluginTopology;
}

/// Produces single-node topology from source topology.
///
/// This routine transforms input topology to a single-node
/// cluster with default tier. This default tier will contain
/// all services from source topology.
///
pub struct SingleNodeTopologyTransformer {
    mctx_provider: Box<dyn MigrationContextProvider>,
}

impl Default for SingleNodeTopologyTransformer {
    fn default() -> Self {
        Self {
            mctx_provider: Box::new(vec![]),
        }
    }
}

impl SingleNodeTopologyTransformer {
    pub fn set_migration_context_provider<P>(&mut self, provider: P)
    where
        P: MigrationContextProvider + 'static,
    {
        self.mctx_provider = Box::new(provider) as Box<_>;
    }
}

impl TopologyTransformer for SingleNodeTopologyTransformer {
    fn transform(&self, source_topology: &PluginTopology) -> PluginTopology {
        let mut topology = source_topology.clone();

        // Use only default single-node tier.
        topology.tiers.clear();
        topology.tiers.insert(
            DEFAULT_TIER.into(),
            Tier {
                replicasets: 1,
                replication_factor: 1,
            },
        );

        // Iterate over plugins in source topology and
        // put their services on default tier.
        for (plugin_name, plugin) in topology.plugins.iter_mut() {
            plugin.migration_context = self.mctx_provider.get_migration_context(plugin_name);
            for (_, service) in plugin.services.iter_mut() {
                service.tiers = vec![DEFAULT_TIER.into()];
            }
        }

        topology
    }
}

#[cfg(test)]
mod tests {

    use crate::topology::{SingleNodeTopologyTransformer, TopologyTransformer, DEFAULT_TIER};
    use pike::cluster::{Plugin, Service, Tier, Topology};
    use rstest::{fixture, rstest};
    use std::collections::BTreeMap;

    #[fixture]
    fn topology() -> Topology {
        let plugins = BTreeMap::from([(
            "test_plugin".to_string(),
            Plugin {
                services: BTreeMap::from([
                    (
                        "storage".to_string(),
                        Service {
                            tiers: vec!["default".to_string()],
                        },
                    ),
                    (
                        "router".to_string(),
                        Service {
                            tiers: vec!["extra".to_string()],
                        },
                    ),
                ]),
                ..Default::default()
            },
        )]);

        let tiers = BTreeMap::from([
            (
                "extra".to_string(),
                Tier {
                    replicasets: 3,
                    replication_factor: 2,
                },
            ),
            (
                "default".to_string(),
                Tier {
                    replicasets: 2,
                    replication_factor: 2,
                },
            ),
        ]);

        let enviroment = BTreeMap::from([("key".to_string(), "value".to_string())]);

        Topology {
            tiers,
            plugins,
            enviroment,
            ..Default::default()
        }
    }

    #[rstest]
    fn test_single_node_topology_transformer(topology: Topology) {
        let transformed = SingleNodeTopologyTransformer::default().transform(&topology);

        let default_tier = transformed.tiers.get(DEFAULT_TIER).unwrap();

        assert_eq!(
            1,
            transformed.tiers.len(),
            "should contain only default tier"
        );
        assert_eq!(1, default_tier.replicasets);
        assert_eq!(1, default_tier.replication_factor);

        let plugin = transformed.plugins.get("test_plugin").unwrap();

        assert_eq!(2, plugin.services.len(), "should contain two services");

        for (_, service) in plugin.services.iter() {
            assert_eq!(vec![DEFAULT_TIER], service.tiers);
        }

        assert_eq!(
            "value",
            transformed.enviroment.get("key").unwrap(),
            "env should've not changed"
        );
    }
}
