use std::cmp::Reverse;

use bevy::{ecs::component::ComponentId, prelude::*};

use crate::core::replication_fns::ReplicationFns;

use super::replication_fns::command_fns::{RemoveFn, WriteFn};

/// Marker-based functions for [`App`].
///
/// Allows to customize behavior on client when receiving an update for server.
///
/// Mostly needed for third-party crates then for end-users.
pub trait AppMarkerExt {
    /// Registers component as a marker.
    ///
    /// Can be used to override how component will be written or removed based on marker presence.
    /// For details see [`Self::register_marker_fns`].
    fn register_marker<M: Component>(&mut self, priority: usize) -> &mut Self;

    /**
    Associates command functions with a marker.

    If this component is present on an entity and its priority is the highest,
    then these functions will be called for this component during replication
    instead of default [`write`](super::replicaton_fns::write) and
    [`remove`](super::replicaton_fns::remove).

    # Examples

    In this example we write all received updates for [`Transform`] into user's
    `ComponentHistory<Transform>` if it present.

    ```
    use bevy::prelude::*;
    use bevy_replicon::{
        client::client_mapper::ServerEntityMap,
        core::replication_fns::{self, ComponentFns, command_fns},
        prelude::*,
    };

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.register_marker::<ComponentMarker>();
    app.register_marker_fns::<Transform, ComponentHistory<Transform>>(
        write_history,
        command_fns::remove,
    );

    /// Instead of writing into a component directly, it writes data into [`ComponentHistory<C>`].
    unsafe fn write_history<C: Component>(
        serde_fns: &SerdeFns,
        commands: &mut Commands,
        entity: &mut EntityMut,
        cursor: &mut Cursor<&[u8]>,
        entity_map: &mut ServerEntityMap,
        _replicon_tick: RepliconTick,
    ) -> bincode::Result<()> {
        let mut mapper = ClientMapper {
            commands,
            entity_map,
        };

        let component: C = serde_fns.deserialize(cursor, &mut mapper)?;
        if let Some(mut history) = entity.get_mut::<ComponentHistory<C>>() {
            history.push(component);
        } else {
            commands.entity(entity.id()).insert(ComponentHistory(vec![component]));
        }

        Ok(())
    }

    /// Stores history of values of `C` received from server. Present only on client.
    ///
    /// In this example, we use it as both a marker and storage.
    #[derive(Component, Deref, DerefMut)]
    struct ComponentHistory<C>(Vec<C>);
    ```
    **/
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

/// Registered markers that override functions if present for
/// [`CommandFns`](super::replication_fns::command_fns::CommandFns).
#[derive(Resource, Default)]
pub(crate) struct CommandMarkers(Vec<CommandMarker>);

impl CommandMarkers {
    /// Inserts a new marker, maintaining sorting by their priority in descending order.
    ///
    /// Use [`ReplicationFns::register_marker`] to register a slot for command functions for this marker.
    fn insert(&mut self, marker: CommandMarker) -> CommandMarkerId {
        let index = self
            .0
            .binary_search_by_key(&Reverse(marker.priority), |marker| Reverse(marker.priority))
            .unwrap_or_else(|index| index);

        self.0.insert(index, marker);

        CommandMarkerId(index)
    }

    /// Returns marker ID from its component ID.
    fn marker_id(&self, component_id: ComponentId) -> CommandMarkerId {
        let index = self
            .0
            .iter()
            .position(|marker| marker.component_id == component_id)
            .unwrap_or_else(|| panic!("marker {component_id:?} wasn't registered"));

        CommandMarkerId(index)
    }

    /// Returns an iterator over markers presence for an entity.
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

/// Unique marker ID.
///
/// Can be obtained from [`CommandMarkers::insert`].
#[derive(Clone, Copy, Deref, Debug)]
pub(super) struct CommandMarkerId(usize);
