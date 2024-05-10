pub mod command_fns;
pub mod component_fns;
pub mod ctx;
pub mod rule_fns;
pub mod test_fns;

use bevy::{ecs::component::ComponentId, prelude::*};
use serde::{Deserialize, Serialize};

use super::command_markers::CommandMarkerIndex;
use command_fns::{RemoveFn, UntypedCommandFns, WriteFn};
use component_fns::ComponentFns;
use ctx::DespawnCtx;
use rule_fns::{RuleFns, UntypedRuleFns};

/// Stores configurable replication functions.
#[derive(Resource)]
pub struct ReplicationFns {
    /// Custom function to handle entity despawning.
    ///
    /// By default uses [`despawn_recursive`].
    /// Useful if you need to intercept despawns and handle them in a special way.
    pub despawn: DespawnFn,

    /// Functions for replicated components.
    ///
    /// Unique for each component.
    components: Vec<(ComponentFns, ComponentId)>,

    /// Serialization/deserialization functions for a component and
    /// the component's index in [`Self::components`].
    ///
    /// Can be registered multiple times for the same component for a different
    /// [`ReplicationRule`](super::replication_rules::ReplicationRule)
    rules: Vec<(UntypedRuleFns, usize)>,

    /// Number of registered markers.
    ///
    /// Used to initialize new [`ComponentFns`] with the registered number of slots.
    marker_slots: usize,
}

impl ReplicationFns {
    /// Registers marker slot for component functions.
    ///
    /// Should be used after calling
    /// [`CommandMarkers::insert`](super::command_markers::CommandMarkers::insert)
    pub(super) fn register_marker(&mut self, marker_id: CommandMarkerIndex) {
        self.marker_slots += 1;
        for (command_fns, _) in &mut self.components {
            command_fns.add_marker_slot(marker_id);
        }
    }

    /// Associates command functions with a marker for a component.
    ///
    /// **Must** be called **after** calling [`Self::register_marker`] with `marker_id`.
    ///
    /// See also [`Self::set_command_fns`].
    ///
    /// # Panics
    ///
    /// Panics if the marker wasn't registered. Use [`Self::register_marker`] first.
    pub(super) fn set_marker_fns<C: Component>(
        &mut self,
        world: &mut World,
        marker_id: CommandMarkerIndex,
        write: WriteFn<C>,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_component_fns::<C>(world);
        let (component_fns, _) = &mut self.components[index];
        let command_fns = UntypedCommandFns::new(write, remove);

        // SAFETY: `component_fns` and `command_fns` were created for `C`.
        unsafe {
            component_fns.set_marker_fns(marker_id, command_fns);
        }
    }

    /// Sets default functions for a component when there are no markers.
    ///
    /// See also [`Self::set_marker_fns`].
    pub(super) fn set_command_fns<C: Component>(
        &mut self,
        world: &mut World,
        write: WriteFn<C>,
        remove: RemoveFn,
    ) {
        let (index, _) = self.init_component_fns::<C>(world);
        let (component_fns, _) = &mut self.components[index];
        let command_fns = UntypedCommandFns::new(write, remove);

        // SAFETY: `component_fns` and `command_fns` were created for `C`.
        unsafe {
            component_fns.set_command_fns(command_fns);
        }
    }

    /// Registers serialization/deserialization functions for a component.
    ///
    /// Returned data can be assigned to a
    /// [`ReplicationRule`](super::replication_rules::ReplicationRule)
    pub fn register_rule_fns<C: Component>(
        &mut self,
        world: &mut World,
        rule_fns: RuleFns<C>,
    ) -> FnsInfo {
        let (index, component_id) = self.init_component_fns::<C>(world);
        self.rules.push((rule_fns.into(), index));

        FnsInfo {
            component_id,
            fns_id: FnsId(self.rules.len() - 1),
        }
    }

    /// Initializes [`ComponentFns`] for a component and returns its index and ID.
    ///
    /// If a [`ComponentFns`] has already been created for this component,
    /// then it returns its index instead of creating a new one.
    fn init_component_fns<C: Component>(&mut self, world: &mut World) -> (usize, ComponentId) {
        let component_id = world.init_component::<C>();
        let index = self
            .components
            .iter()
            .position(|&(_, id)| id == component_id)
            .unwrap_or_else(|| {
                self.components
                    .push((ComponentFns::new::<C>(self.marker_slots), component_id));
                self.components.len() - 1
            });

        (index, component_id)
    }

    /// Returns associates functions.
    ///
    /// See also [`Self::register_rule_fns`].
    pub(crate) fn get(&self, fns_id: FnsId) -> (&ComponentFns, &UntypedRuleFns) {
        let (rule_fns, index) = self
            .rules
            .get(fns_id.0)
            .expect("serde function IDs should be obtained from the same instance");

        // SAFETY: index obtained from `rules` is always valid.
        let (command_fns, _) = unsafe { self.components.get_unchecked(*index) };

        (command_fns, rule_fns)
    }
}

impl Default for ReplicationFns {
    fn default() -> Self {
        Self {
            despawn: despawn_recursive,
            components: Default::default(),
            rules: Default::default(),
            marker_slots: 0,
        }
    }
}

/// IDs of a registered replication function and its component.
///
/// Can be obtained from [`ReplicationFns::register_rule_fns`].
#[derive(Clone, Copy)]
pub struct FnsInfo {
    component_id: ComponentId,
    fns_id: FnsId,
}

impl FnsInfo {
    pub(crate) fn component_id(&self) -> ComponentId {
        self.component_id
    }

    pub(crate) fn fns_id(&self) -> FnsId {
        self.fns_id
    }
}

/// ID of replicaton functions for a component.
///
/// Can be obtained from [`ReplicationFns::register_rule_fns`].
#[derive(Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct FnsId(usize);

/// Signature of the entity despawn function.
pub type DespawnFn = fn(&DespawnCtx, EntityWorldMut);

/// Default entity despawn function.
pub fn despawn_recursive(_ctx: &DespawnCtx, entity: EntityWorldMut) {
    entity.despawn_recursive();
}

#[cfg(test)]
mod tests {
    use bevy::ecs::entity::MapEntities;

    use super::*;

    #[test]
    fn rule_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_rule_fns(&mut world, RuleFns::<ComponentA>::default());
        assert_eq!(replication_fns.rules.len(), 1);
        assert_eq!(replication_fns.components.len(), 1);
    }

    #[test]
    fn duplicate_rule_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_rule_fns(&mut world, RuleFns::<ComponentA>::default());
        replication_fns.register_rule_fns(&mut world, RuleFns::<ComponentA>::default());

        assert_eq!(replication_fns.rules.len(), 2);
        assert_eq!(
            replication_fns.components.len(),
            1,
            "multiple serde registrations for the same component should result only in a single command functions instance"
        );
    }

    #[test]
    fn different_rule_fns() {
        let mut world = World::new();
        let mut replication_fns = ReplicationFns::default();
        replication_fns.register_rule_fns(&mut world, RuleFns::<ComponentA>::default());
        replication_fns.register_rule_fns(&mut world, RuleFns::<ComponentB>::default());

        assert_eq!(replication_fns.rules.len(), 2);
        assert_eq!(replication_fns.components.len(), 2);
    }

    #[derive(Component, Serialize, Deserialize)]
    struct ComponentA;

    #[derive(Component, Deserialize, Serialize)]
    struct ComponentB;

    impl MapEntities for ComponentB {
        fn map_entities<M: EntityMapper>(&mut self, _entity_mapper: &mut M) {}
    }
}
