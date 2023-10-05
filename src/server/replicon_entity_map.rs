use bevy::{
    prelude::{Entity, Resource},
    utils::hashbrown::HashMap,
};

use super::RepliconTick;
/// ['ClientEntityMap'] is a resource that exists on the server for mapping server entities to
/// entities that clients have already spawned. The mappings are sent to clients and injected into
/// the client's [`crate::client::NetworkEntityMap`].
///
/// Sometimes you don't want to wait for the server to spawn something before it appears on the
/// client – when a client presses shoot, they can immediately spawn the bullet, then match up that
/// entity with the eventual replicated bullet the server spawns, rather than have replication spawn
/// a brand new bullet on the client.
///
/// In this situation, the server can write the client `Entity` it sent (in your game's custom
/// protocol) into the [`ClientEntityMap`], associating it with the newly spawned server entity.
///
/// Replication packets will send a list of such mappings
/// to clients, which will be inserted into the client's [`crate::client::NetworkEntityMap`].
///
/// ### Example usage:
///
/// ```rust,ignore
/// // on client:
/// if (pressed_shoot()) {
///     let client_predicted_entity = commands.spawn((Bullet, Prediction)).id();
///     // your game's netcode sends the client entity along with the shoot command:
///     send_shoot_command_to_server(client_predicted_entity);
/// }
/// // on server:
/// fn apply_inputs_system(mut entity_map: ResMut<ClientEntityMap>, tick: Res<RepliconTick>) {
///     // ...
///     if player_input.pressed_shoot() {
///         let server_entity = commands.spawn((Bullet, Replication, Etc)).id();
///         // your game's netcode checks for a client predicted entity, and registers it here:
///         entity_map.insert(
///             player_input.client_id,
///             server_entity,
///             player_input.client_predicted_entity,
///             tick
///         );
///     }
/// }
/// ```
///
/// Provided that `client_predicted_entity` exists when the replication data for `server_entity`
/// arrives, replicated data will be applied to that entity instead of spawning a new one.
/// You can detect when this happens by querying for `Added<Replication>` on your client entity.
///
/// If `client_predicted_entity` is not found, a new entity will be spawned on the client,
/// just the same as when no client prediction is provided.
///
#[derive(Resource, Debug, Default)]
pub struct ClientEntityMap {
    mappings: HashMap<u64, Vec<EntityMapping>>,
}
pub(crate) type EntityMapping = (RepliconTick, ServerEntity, ClientEntity);

// Aliases for clarity in APIs dealing with `Entity`s that exist on servers and clients.
pub(crate) type ServerEntity = Entity;
pub(crate) type ClientEntity = Entity;

impl ClientEntityMap {
    /// Register that the server spawned `server_entity` as a result of `client_id` sending a
    /// command which included a `client_entity` they already spawned. This will be sent and added
    /// to the client's [`crate::client::NetworkEntityMap`].
    ///
    /// The current `tick` is needed so that this prediction data can be cleaned up once the tick
    /// has been acked by the client.
    pub fn insert(
        &mut self,
        client_id: u64,
        server_entity: Entity,
        client_entity: Entity,
        tick: RepliconTick,
    ) {
        let new_entry = (tick, server_entity, client_entity);
        if let Some(v) = self.mappings.get_mut(&client_id) {
            v.push(new_entry);
        } else {
            self.mappings.insert(client_id, vec![new_entry]);
        }
    }
    /// Get entity mappings for a client that have been added since the `from_tick`.
    pub(crate) fn get_mappings(
        &self,
        client_id: u64,
        from_tick: RepliconTick,
    ) -> Option<impl Iterator<Item = (&ServerEntity, &ClientEntity)> + '_> {
        let Some(v) = self.mappings.get(&client_id) else {
            return None;
        };
        Some(
            v.iter()
                .filter_map(move |(entry_tick, server_entity, client_entity)| {
                    if *entry_tick >= from_tick {
                        Some((server_entity, client_entity))
                    } else {
                        None
                    }
                }),
        )
    }
    /// remove predicted entities in cases where the RepliconTick at which that entity mapping
    /// was created has been acked by the client.
    pub(crate) fn cleanup_acked(&mut self, client_id: u64, acked_tick: RepliconTick) {
        let Some(v) = self.mappings.get_mut(&client_id) else {
            return;
        };
        v.retain(|(tick, _, _)| *tick > acked_tick);
    }
}
