use std::{io::Cursor, mem};

use bevy::{
    ecs::{
        archetype::{ArchetypeGeneration, Archetypes},
        component::ComponentId,
        entity::MapEntities,
        event::ManualEventReader,
        removal_detection::{RemovedComponentEntity, RemovedComponentEvents},
    },
    prelude::*,
    ptr::Ptr,
    utils::HashMap,
};
use bincode::{DefaultOptions, Options};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    client::client_mapper::{ClientMapper, ServerEntityMap},
    core::{
        replicated_archetypes::{ReplicatedArchetype, ReplicatedArchetypes, ReplicatedComponent},
        replication_fns::{
            DeserializeFn, RemoveFn, RemoveFnId, ReplicationFns, SerdeFns, SerdeFnsId, SerializeFn,
        },
        replicon_tick::RepliconTick,
    },
    server::world_buffers::{DespawnBuffer, RemovalBuffer},
    server_running, ServerSet,
};

pub struct ComponentRulesPlugin;

impl Plugin for ComponentRulesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ComponentRules>().add_systems(
            PostUpdate,
            (
                Self::update_replicated_archetypes.in_set(ServerSet::UpdateArchetypes),
                Self::buffer_removals
                    .in_set(ServerSet::BufferRemovals)
                    .run_if(server_running),
            ),
        );
    }
}

impl ComponentRulesPlugin {
    fn update_replicated_archetypes(
        archetypes: &Archetypes,
        mut replicated_archetypes: ResMut<ReplicatedArchetypes>,
        mut component_rules: ResMut<ComponentRules>,
    ) {
        let old_generation = component_rules.update_generation(archetypes);

        // Archetypes are never removed, iterate over newly added since the last update.
        let marker_id = replicated_archetypes.marker_id();
        for archetype in archetypes[old_generation..]
            .iter()
            .filter(|archetype| archetype.contains(marker_id))
        {
            let mut replicated_archetype = ReplicatedArchetype::new(archetype.id());
            for component_id in archetype.components() {
                let Some(&serde_id) = component_rules.serde_ids().get(&component_id) else {
                    continue;
                };

                // SAFETY: component ID obtained from this archetype.
                let storage_type =
                    unsafe { archetype.get_storage_type(component_id).unwrap_unchecked() };

                let replicated_component = ReplicatedComponent {
                    component_id,
                    storage_type,
                    serde_id,
                };

                // SAFETY: Component ID and storage type obtained from this archetype,
                // serde functions ID points to existing functions from `ComponentRules`.
                unsafe { replicated_archetype.add_component(replicated_component) };
            }

            // SAFETY: Archetype ID corresponds to a valid archetype.
            unsafe { replicated_archetypes.add_archetype(replicated_archetype) };
        }
    }

    fn buffer_removals(
        mut readers: Local<HashMap<ComponentId, ManualEventReader<RemovedComponentEntity>>>,
        remove_events: &RemovedComponentEvents,
        mut removal_buffer: ResMut<RemovalBuffer>,
        component_rules: Res<ComponentRules>,
        despawn_buffer: Res<DespawnBuffer>,
    ) {
        for (&component_id, &serde_id) in component_rules.remove_ids() {
            for removals in remove_events.get(component_id).into_iter() {
                let reader = readers.entry(component_id).or_default();
                for entity in reader
                    .read(removals)
                    .cloned()
                    .map(Into::into)
                    .filter(|entity| !despawn_buffer.contains(entity))
                {
                    removal_buffer.insert(entity, serde_id);
                }
            }
        }
    }
}

pub trait AppReplicationExt {
    /// Marks component for replication.
    ///
    /// Component will be serialized as is using bincode.
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned;

    /// Same as [`Self::replicate`], but additionally maps server entities to client inside the component after receiving.
    ///
    /// Always use it for components that contain entities.
    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapEntities;

    /// Same as [`Self::replicate`], but uses the specified functions for serialization, deserialization, and removal.
    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveFn,
    ) -> &mut Self
    where
        C: Component;
}

impl AppReplicationExt for App {
    fn replicate<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned,
    {
        self.replicate_with::<C>(
            serialize_component::<C>,
            deserialize_component::<C>,
            remove_component::<C>,
        )
    }

    fn replicate_mapped<C>(&mut self) -> &mut Self
    where
        C: Component + Serialize + DeserializeOwned + MapEntities,
    {
        self.replicate_with::<C>(
            serialize_component::<C>,
            deserialize_mapped_component::<C>,
            remove_component::<C>,
        )
    }

    fn replicate_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveFn,
    ) -> &mut Self
    where
        C: Component,
    {
        let component_id = self.world.init_component::<C>();
        let serde_fns = SerdeFns {
            serialize,
            deserialize,
        };

        let mut replication_fns = self.world.resource_mut::<ReplicationFns>();
        let serde_id = replication_fns.add_serde_fns(serde_fns);
        let remove_id = replication_fns.add_remove_fn(remove);

        let mut component_rules = self.world.resource_mut::<ComponentRules>();
        component_rules.serde_ids.insert(component_id, serde_id);
        component_rules.remove_ids.insert(component_id, remove_id);

        self
    }
}

/// Stores information about which components will be replicated.
#[derive(Resource)]
pub struct ComponentRules {
    /// Maps component IDs to their serde functions IDs.
    serde_ids: HashMap<ComponentId, SerdeFnsId>,

    /// Maps component IDs to their remove function IDs.
    remove_ids: HashMap<ComponentId, RemoveFnId>,

    /// Highest processed archetype ID.
    ///
    /// See also [`Self::update_generation`].
    generation: ArchetypeGeneration,
}

impl ComponentRules {
    /// Replaces stored generation with the highest archetype ID and returns previous.
    ///
    /// This should be used to iterate over newly introduced
    /// [`Archetype`](bevy::ecs::archetype::Archetype)s since the last time this function was called.
    pub(crate) fn update_generation(&mut self, archetypes: &Archetypes) -> ArchetypeGeneration {
        mem::replace(&mut self.generation, archetypes.generation())
    }

    /// Returns mapping of replicated components to their serde function IDs.
    #[must_use]
    pub(crate) fn serde_ids(&self) -> &HashMap<ComponentId, SerdeFnsId> {
        &self.serde_ids
    }

    /// Returns mapping of replicated components to their remove function IDs.
    #[must_use]
    pub(crate) fn remove_ids(&self) -> &HashMap<ComponentId, RemoveFnId> {
        &self.remove_ids
    }
}

impl Default for ComponentRules {
    fn default() -> Self {
        Self {
            serde_ids: Default::default(),
            remove_ids: Default::default(),
            generation: ArchetypeGeneration::initial(),
        }
    }
}

/// Default serialization function.
pub fn serialize_component<C: Component + Serialize>(
    component: Ptr,
    cursor: &mut Cursor<Vec<u8>>,
) -> bincode::Result<()> {
    // SAFETY: Function called for registered `ComponentId`.
    let component: &C = unsafe { component.deref() };
    DefaultOptions::new().serialize_into(cursor, component)
}

/// Default deserialization function.
pub fn deserialize_component<C: Component + DeserializeOwned>(
    entity: &mut EntityWorldMut,
    _entity_map: &mut ServerEntityMap,
    cursor: &mut Cursor<&[u8]>,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let component: C = DefaultOptions::new().deserialize_from(cursor)?;
    entity.insert(component);

    Ok(())
}

/// Like [`deserialize_component`], but also maps entities before insertion.
pub fn deserialize_mapped_component<C: Component + DeserializeOwned + MapEntities>(
    entity: &mut EntityWorldMut,
    entity_map: &mut ServerEntityMap,
    cursor: &mut Cursor<&[u8]>,
    _replicon_tick: RepliconTick,
) -> bincode::Result<()> {
    let mut component: C = DefaultOptions::new().deserialize_from(cursor)?;

    entity.world_scope(|world| {
        component.map_entities(&mut ClientMapper::new(world, entity_map));
    });

    entity.insert(component);

    Ok(())
}

/// Default component removal function.
pub fn remove_component<C: Component>(entity: &mut EntityWorldMut, _replicon_tick: RepliconTick) {
    entity.remove::<C>();
}
