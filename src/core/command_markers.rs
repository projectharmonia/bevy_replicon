use std::cmp::Reverse;

use bevy::{ecs::component::ComponentId, prelude::*};

use crate::core::replication_fns::ReplicationFns;

use super::replication_fns::command_fns::{RemoveFn, WriteFn};

/// Marker-based functions for [`App`].
///
/// Allows customizing behavior on clients when receiving updates from the server.
///
/// We check markers on receive instead of archetypes because on client we don't
/// know an incoming entity's archetype in advance.
///
/// This is mostly needed for third-party crates, most end-users should not need to use it directly.
pub trait AppMarkerExt {
    /// Registers a component as a marker.
    ///
    /// Can be used to override how this component or other components will be written or removed
    /// based on marker-component presence.
    /// For details see [`Self::set_marker_fns`].
    ///
    /// This function registers markers with priority equal to 0.
    /// Use [`Self::register_marker_with_priority`] if you have multiple
    /// markers affecting the same component.
    fn register_marker<M: Component>(&mut self) -> &mut Self;

    /// Same as [`Self::register_marker`], but allows setting a priority.
    fn register_marker_with_priority<M: Component>(&mut self, priority: usize) -> &mut Self;

    /**
    Associates command functions with a marker for a component.

    If this marker is present on an entity and its priority is the highest,
    then these functions will be called for this component during replication
    instead of [`default_write`](super::replication_fns::command_fns::default_write) and
    [`default_remove`](super::replication_fns::command_fns::default_remove).
    See also [`Self::set_command_fns`].

    # Safety

    The caller must ensure that passed `write` can be safely called with a
    [`SerdeFns`](super::replication_fns::serde_fns::SerdeFns) created for `C`.

    # Examples

    In this example we write all received updates for [`Transform`] into user's
    `History<Transform>` if `ComponentsHistory` marker is present on the client entity. In this
    scenario, you'd insert `ComponentsHistory` the first time the entity
    is replicated (e.g. by detecting a `Player` marker component using the blueprint pattern).
    Then [`Transform`] updates after that will be inserted to the history.

    ```
    use std::io::Cursor;

    use bevy::prelude::*;
    use bevy_replicon::{
        client::client_mapper::{ClientMapper, ServerEntityMap},
        core::{
            replication_fns::{command_fns, serde_fns::SerdeFns},
            replicon_tick::RepliconTick,
        },
        prelude::*,
    };

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.register_marker::<ComponentsHistory>();
    // SAFETY: `write_history` can be safely called with a `SerdeFns` created for `Transform`.
    unsafe {
        app.set_marker_fns::<ComponentsHistory, Transform>(
            write_history::<Transform>,
            command_fns::default_remove::<Transform>,
        );
    }

    /// Instead of writing into a component directly, it writes data into [`ComponentHistory<C>`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that `serde_fns` was created for [`Transform`].
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
        if let Some(mut history) = entity.get_mut::<History<C>>() {
            history.push(component);
        } else {
            commands
                .entity(entity.id())
                .insert(History(vec![component]));
        }

        Ok(())
    }

    /// If this marker is present on an entity, registered components will be stored in [`History<T>`].
    ///
    ///Present only on client.
    #[derive(Component)]
    struct ComponentsHistory;

    /// Stores history of values of `C` received from server. Present only on client.
    ///
    ///Present only on client.
    #[derive(Component, Deref, DerefMut)]
    struct History<C>(Vec<C>);
    ```
    **/
    unsafe fn set_marker_fns<M: Component, C: Component>(
        &mut self,
        write: WriteFn,
        remove: RemoveFn,
    ) -> &mut Self;

    /// Sets default functions for a component when there are no markers.
    ///
    /// If there are no markers are present on an entity, then these functions will
    /// be called for this component during replication instead of
    /// [`default_write`](super::replication_fns::command_fns::default_write) and
    /// [`default_remove`](super::replication_fns::command_fns::default_remove).
    /// See also [`Self::set_marker_fns`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that passed `write` can be safely called with a
    /// [`SerdeFns`](super::replication_fns::serde_fns::SerdeFns) created for `C`.
    unsafe fn set_command_fns<C: Component>(
        &mut self,
        write: WriteFn,
        remove: RemoveFn,
    ) -> &mut Self;
}

impl AppMarkerExt for App {
    fn register_marker<M: Component>(&mut self) -> &mut Self {
        self.register_marker_with_priority::<M>(0)
    }

    fn register_marker_with_priority<M: Component>(&mut self, priority: usize) -> &mut Self {
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

    unsafe fn set_marker_fns<M: Component, C: Component>(
        &mut self,
        write: WriteFn,
        remove: RemoveFn,
    ) -> &mut Self {
        let component_id = self.world.init_component::<M>();
        let command_markers = self.world.resource::<CommandMarkers>();
        let marker_id = command_markers.marker_id(component_id);
        self.world
            .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| unsafe {
                // SAFETY: The caller ensured that `write` can be safely called with a `SerdeFns` created for `C`.
                replication_fns.set_marker_fns::<C>(world, marker_id, write, remove);
            });

        self
    }

    unsafe fn set_command_fns<C: Component>(
        &mut self,
        write: WriteFn,
        remove: RemoveFn,
    ) -> &mut Self {
        self.world
            .resource_scope(|world, mut replication_fns: Mut<ReplicationFns>| unsafe {
                // SAFETY: The caller ensured that `write` can be safely called with a `SerdeFns` created for `C`.
                replication_fns.set_command_fns::<C>(world, write, remove);
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
    pub(super) fn insert(&mut self, marker: CommandMarker) -> CommandMarkerId {
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

/// Component marker information.
///
/// See also [`CommandMarkers`].
pub(super) struct CommandMarker {
    /// Marker ID.
    pub(super) component_id: ComponentId,

    /// Priority of this marker.
    ///
    /// Will affect the order in [`CommandMarkers::insert`].
    pub(super) priority: usize,
}

/// Unique marker ID.
///
/// Can be obtained from [`CommandMarkers::insert`].
#[derive(Clone, Copy, Deref, Debug)]
pub(super) struct CommandMarkerId(usize);

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::core::replication_fns::{command_fns, ReplicationFns};

    #[test]
    #[should_panic]
    fn non_registered_marker() {
        let mut app = App::new();
        app.init_resource::<CommandMarkers>()
            .init_resource::<ReplicationFns>();

        // SAFETY: `write` can be safely called with a `SerdeFns` created for `DummyComponent`.
        unsafe {
            app.set_marker_fns::<DummyMarkerA, DummyComponent>(
                command_fns::default_write::<DummyComponent>,
                command_fns::default_remove::<DummyComponent>,
            );
        }
    }

    #[test]
    fn sorting() {
        let mut app = App::new();
        app.init_resource::<CommandMarkers>()
            .init_resource::<ReplicationFns>()
            .register_marker::<DummyMarkerA>()
            .register_marker_with_priority::<DummyMarkerB>(2)
            .register_marker_with_priority::<DummyMarkerC>(1)
            .register_marker::<DummyMarkerD>();

        let markers = app.world.resource::<CommandMarkers>();
        let priorities: Vec<_> = markers.0.iter().map(|marker| marker.priority).collect();
        assert_eq!(priorities, [2, 1, 0, 0]);
    }

    #[derive(Component)]
    struct DummyMarkerA;

    #[derive(Component)]
    struct DummyMarkerB;

    #[derive(Component)]
    struct DummyMarkerC;

    #[derive(Component)]
    struct DummyMarkerD;

    #[derive(Component, Serialize, Deserialize)]
    struct DummyComponent;
}
