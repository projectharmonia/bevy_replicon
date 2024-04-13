use std::cmp::Reverse;

use bevy::{ecs::component::ComponentId, prelude::*};

use crate::core::replication_fns::ReplicationFns;

use super::replication_fns::command_fns::{RemoveFn, WriteFn};

/// Marker functions for [`App`].
pub trait AppMarkerExt {
    fn register_marker<M: Component>(&mut self, priority: usize) -> &mut Self;
    fn register_marker_fns<C: Component, M: Component>(
        &mut self,
        write: WriteFn,
        remove: RemoveFn,
    ) -> &mut Self;
}

impl AppMarkerExt for App {
    fn register_marker<M: Component>(&mut self, priority: usize) -> &mut Self {
        let component_id = self.world.init_component::<M>();
        let mut command_markers = self.world.resource_mut::<CommandMarkers>();
        let marker_id = command_markers.insert(CommandMarker {
            component_id,
            priority,
        });

        let mut replicaton_fns = self.world.resource_mut::<ReplicationFns>();
        replicaton_fns.register_marker(marker_id);

        self
    }

    fn register_marker_fns<C: Component, M: Component>(
        &mut self,
        write: WriteFn,
        remove: RemoveFn,
    ) -> &mut Self {
        let component_id = self.world.init_component::<M>();
        let command_markers = self.world.resource::<CommandMarkers>();
        let marker_id = command_markers.marker_id(component_id);
        self.world
            .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| {
                replication_fns.register_marker_fns::<C>(world, marker_id, write, remove);
            });

        self
    }
}

#[derive(Resource, Default)]
pub(crate) struct CommandMarkers(Vec<CommandMarker>);

impl CommandMarkers {
    fn insert(&mut self, marker: CommandMarker) -> CommandMarkerId {
        let index = self
            .0
            .binary_search_by_key(&Reverse(marker.priority), |marker| Reverse(marker.priority))
            .unwrap_or_else(|index| index);

        self.0.insert(index, marker);

        CommandMarkerId(index)
    }

    fn marker_id(&self, component_id: ComponentId) -> CommandMarkerId {
        let index = self
            .0
            .iter()
            .position(|marker| marker.component_id == component_id)
            .unwrap_or_else(|| panic!("marker {component_id:?} wasn't registered"));

        CommandMarkerId(index)
    }

    pub(crate) fn iter_contains<'a>(
        &'a self,
        entity: &'a EntityMut,
    ) -> impl Iterator<Item = bool> + 'a {
        self.0
            .iter()
            .map(move |marker| entity.contains_id(marker.component_id))
    }
}

struct CommandMarker {
    component_id: ComponentId,
    priority: usize,
}

#[derive(Clone, Copy, Deref, Debug)]
pub(super) struct CommandMarkerId(usize);
