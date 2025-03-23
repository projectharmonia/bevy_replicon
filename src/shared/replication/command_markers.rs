use std::cmp::Reverse;

use bevy::{ecs::component::ComponentId, prelude::*};

use super::replication_registry::{
    ReplicationRegistry,
    command_fns::{RemoveFn, WriteFn},
};

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
    /// This function registers markers with default [`MarkerConfig`].
    /// See also [`Self::register_marker_with`].
    fn register_marker<M: Component>(&mut self) -> &mut Self;

    /// Same as [`Self::register_marker`], but also accepts marker configuration.
    fn register_marker_with<M: Component>(&mut self, config: MarkerConfig) -> &mut Self;

    /**
    Associates command functions with a marker for a component.

    If this marker is present on an entity and its priority is the highest,
    then these functions will be called for this component during replication
    instead of [`default_write`](super::replication_registry::command_fns::default_write) and
    [`default_remove`](super::replication_registry::command_fns::default_remove).
    See also [`Self::set_command_fns`].

    # Examples

    In this example we write all received updates for [`Transform`] into user's
    `History<Transform>` if `History` marker is present on the client entity. In this
    scenario, you'd insert `History` the first time the entity
    is replicated (e.g. by detecting a `Player` marker component using the blueprint pattern).
    Then [`Transform`] updates after that will be inserted to the history.

    ```
    use bevy::{ecs::system::EntityCommands, prelude::*, utils::HashMap};
    use bevy_replicon::{
        bytes::Bytes,
        shared::{
            replication::{
                command_markers::MarkerConfig,
                deferred_entity::DeferredEntity,
                replication_registry::{
                    ctx::{RemoveCtx, WriteCtx},
                    rule_fns::RuleFns,
                },
            },
            replicon_tick::RepliconTick,
        },
        prelude::*,
    };

    # let mut app = App::new();
    # app.add_plugins(RepliconPlugins);
    app.register_marker_with::<ComponentsHistory>(MarkerConfig {
        need_history: true, // Enable writing for values that are older than the last received value.
        ..Default::default()
    })
    .set_marker_fns::<ComponentsHistory, Transform>(write_history, remove_history::<Transform>);

    /// Instead of writing into a component directly, it writes data into [`History<C>`].
    fn write_history<C: Component>(
        ctx: &mut WriteCtx,
        rule_fns: &RuleFns<C>,
        entity: &mut DeferredEntity,
        message: &mut Bytes,
    ) -> postcard::Result<()> {
        let component: C = rule_fns.deserialize(ctx, message)?;
        if let Some(mut history) = entity.get_mut::<History<C>>() {
            history.insert(ctx.message_tick, component);
        } else {
            ctx.commands
                .entity(entity.id())
                .insert(History([(ctx.message_tick, component)].into()));
        }

        Ok(())
    }

    /// Removes component `C` and its history.
    fn remove_history<C: Component>(ctx: &mut RemoveCtx, entity: &mut DeferredEntity) {
        ctx.commands.entity(entity.id()).remove::<History<C>>().remove::<C>();
    }

    /// If this marker is present on an entity, registered components will be stored in [`History<T>`].
    ///
    ///Present only on client.
    #[derive(Component)]
    struct ComponentsHistory;

    /// Stores history of values of `C` received from server. Present only on client.
    ///
    /// Present only on client.
    #[derive(Component, Deref, DerefMut)]
    struct History<C>(HashMap<RepliconTick, C>);
    ```
    **/
    fn set_marker_fns<M: Component, C: Component>(
        &mut self,
        write: WriteFn<C>,
        remove: RemoveFn,
    ) -> &mut Self;

    /// Sets default functions for a component when there are no markers.
    ///
    /// If there are no markers present on an entity, then these functions will
    /// be called for this component during replication instead of
    /// [`default_write`](super::replication_registry::command_fns::default_write) and
    /// [`default_remove`](super::replication_registry::command_fns::default_remove).
    /// See also [`Self::set_marker_fns`].
    fn set_command_fns<C: Component>(&mut self, write: WriteFn<C>, remove: RemoveFn) -> &mut Self;
}

impl AppMarkerExt for App {
    fn register_marker<M: Component>(&mut self) -> &mut Self {
        self.register_marker_with::<M>(MarkerConfig::default())
    }

    fn register_marker_with<M: Component>(&mut self, config: MarkerConfig) -> &mut Self {
        let component_id = self.world_mut().register_component::<M>();
        let mut command_markers = self.world_mut().resource_mut::<CommandMarkers>();
        let marker_id = command_markers.insert(CommandMarker {
            component_id,
            config,
        });

        let mut replicaton_fns = self.world_mut().resource_mut::<ReplicationRegistry>();
        replicaton_fns.register_marker(marker_id);

        self
    }

    fn set_marker_fns<M: Component, C: Component>(
        &mut self,
        write: WriteFn<C>,
        remove: RemoveFn,
    ) -> &mut Self {
        let component_id = self.world_mut().register_component::<M>();
        let command_markers = self.world().resource::<CommandMarkers>();
        let marker_id = command_markers.marker_id(component_id);
        self.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.set_marker_fns::<C>(world, marker_id, write, remove);
            });

        self
    }

    fn set_command_fns<C: Component>(&mut self, write: WriteFn<C>, remove: RemoveFn) -> &mut Self {
        self.world_mut()
            .resource_scope(|world, mut registry: Mut<ReplicationRegistry>| {
                registry.set_command_fns::<C>(world, write, remove);
            });

        self
    }
}

/// Registered markers that override command functions if present.
#[derive(Resource, Default)]
pub(crate) struct CommandMarkers(Vec<CommandMarker>);

impl CommandMarkers {
    /// Inserts a new marker, maintaining sorting by their priority in descending order.
    ///
    /// May invalidate previously returned [`CommandMarkerIndex`] due to sorting.
    ///
    /// Use [`ReplicationRegistry::register_marker`] to register a slot for command functions for this marker.
    fn insert(&mut self, marker: CommandMarker) -> CommandMarkerIndex {
        let key = Reverse(marker.config.priority);
        let index = self
            .0
            .binary_search_by_key(&key, |marker| Reverse(marker.config.priority))
            .unwrap_or_else(|index| index);

        self.0.insert(index, marker);

        CommandMarkerIndex(index)
    }

    /// Returns marker ID from its component ID.
    fn marker_id(&self, component_id: ComponentId) -> CommandMarkerIndex {
        let index = self
            .0
            .iter()
            .position(|marker| marker.component_id == component_id)
            .unwrap_or_else(|| panic!("marker {component_id:?} wasn't registered"));

        CommandMarkerIndex(index)
    }

    pub(super) fn iter_require_history(&self) -> impl Iterator<Item = bool> + '_ {
        self.0.iter().map(|marker| marker.config.need_history)
    }
}

/// Component marker information.
///
/// See also [`CommandMarkers`].
struct CommandMarker {
    /// Marker ID.
    component_id: ComponentId,

    /// User-registered configuration.
    config: MarkerConfig,
}

/// Parameters for a marker.
#[derive(Default)]
pub struct MarkerConfig {
    /// Priority of this marker.
    ///
    /// All tokens are sorted by priority, and if there are multiple matching
    /// markers, the marker with the highest priority will be used.
    ///
    /// By default set to `0`.
    pub priority: usize,

    /// Represents whether a marker needs to process old mutations.
    ///
    /// Since mutations use [`Channel::Unreliable`](crate::shared::backend::replicon_channels::Channel),
    /// a client may receive an older mutation for an entity component. By default these mutations are discarded,
    /// but some markers may need them. If this field is set to `true`, old component mutations will
    /// be passed to the writing function for this marker.
    ///
    /// By default set to `false`.
    pub need_history: bool,
}

/// Stores which markers are present on an entity.
pub(crate) struct EntityMarkers {
    markers: Vec<bool>,
    need_history: bool,
}

impl EntityMarkers {
    pub(crate) fn read<'a>(
        &'a mut self,
        markers: &CommandMarkers,
        entity: impl Into<EntityRef<'a>>,
    ) {
        self.markers.clear();
        self.need_history = false;

        let entity = entity.into();
        for marker in &markers.0 {
            let contains = entity.contains_id(marker.component_id);
            self.markers.push(contains);
            if contains && marker.config.need_history {
                self.need_history = true;
            }
        }
    }

    /// Returns a slice of which markers are present on an entity.
    ///
    /// Indices corresponds markers in to [`CommandMarkers`].
    pub(super) fn markers(&self) -> &[bool] {
        &self.markers
    }

    /// Returns `true` if an entity has at least one marker that needs history.
    pub(crate) fn need_history(&self) -> bool {
        self.need_history
    }
}

impl FromWorld for EntityMarkers {
    fn from_world(world: &mut World) -> Self {
        let markers = world.resource::<CommandMarkers>();
        Self {
            markers: Vec::with_capacity(markers.0.len()),
            need_history: false,
        }
    }
}

/// Can be obtained from [`CommandMarkers::insert`].
///
/// Shouldn't be stored anywhere since insertion may invalidate old indices.
#[derive(Clone, Copy, Deref, Debug)]
pub(super) struct CommandMarkerIndex(usize);

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::shared::replication::replication_registry::command_fns;

    #[test]
    #[should_panic]
    fn non_registered_marker() {
        let mut app = App::new();
        app.init_resource::<CommandMarkers>()
            .init_resource::<ReplicationRegistry>()
            .set_marker_fns::<DummyMarkerA, DummyComponent>(
                command_fns::default_write,
                command_fns::default_remove::<DummyComponent>,
            );
    }

    #[test]
    fn sorting() {
        let mut app = App::new();
        app.init_resource::<CommandMarkers>()
            .init_resource::<ReplicationRegistry>()
            .register_marker::<DummyMarkerA>()
            .register_marker_with::<DummyMarkerB>(MarkerConfig {
                priority: 2,
                ..Default::default()
            })
            .register_marker_with::<DummyMarkerC>(MarkerConfig {
                priority: 1,
                ..Default::default()
            })
            .register_marker::<DummyMarkerD>();

        let markers = app.world().resource::<CommandMarkers>();
        let priorities: Vec<_> = markers
            .0
            .iter()
            .map(|marker| marker.config.priority)
            .collect();
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
