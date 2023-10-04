use bevy::{
    prelude::{Entity, Resource},
    utils::hashbrown::HashMap,
};

use super::RepliconTick;
/// Tracks client-predicted entities, which are sent along with spawn data when the corresponding
/// entity is created on the server.
///
/// Sometimes you don't want to wait for the server to spawn something before it appears on the
/// client. When a client presses shoot, they can immediately spawn the bullet, then match up that
/// entity with the eventual replicated bullet the server spawns, rather than have replication spawn
/// a brand new bullet on the client.
///
/// ### Example usage:
///
/// Your client presses shoot and spawns a predicted bullet immediately, in anticipation of the
/// server replicating a newly spawned bullet, and matching it up to our predicted entity:
///
/// ```rust,ignore
/// // on client:
/// if (pressed_shoot()) {
///     let client_predicted_entity = commands.spawn((Bullet, Prediction)).id();
///     // your game's netcode sends the client entity along with the shoot command:
///     send_shoot_command_to_server(client_predicted_entity);
/// }
/// // on server:
/// fn apply_inputs_system(mut predictions: ResMut<PredictionTracker>, tick: Res<RepliconTick>) {
///     // ...
///     if player_input.pressed_shoot() {
///         let server_entity = commands.spawn((Bullet, Replication, Etc)).id();
///         // your game's netcode checks for a client predicted entity, and registers it here:
///         predictions.insert(
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
///
/// If `client_predicted_entity` is not found, a new entity will be spawned on the client,
/// just the same as when no client prediction is provided.
///
/// ### Successful prediction detection
///
/// Upon successful replication, the predicted client entity will receive the Replication component.
///
/// Check for this in a system to perform cleanup:
///
/// ```rust
/// fn cleanup_successful_predictions(
///     q: Query<Entity, (With<Prediction>, Added<Replication>)>,
///     mut commands: Commands,
/// ) {
///     for entity in q.iter() {
///         commands.entity(entity).remove::<Prediction>();
///     }
/// }
/// ```
///
/// Typically your Prediction marker component might include a TTL or timeout, after which the
/// predicted entity would be despawned by your game's misprediction cleanup system.
///
#[derive(Resource, Debug, Default)]
pub struct PredictionTracker {
    mappings: HashMap<u64, Vec<EntityMapping>>,
}
type EntityMapping = (RepliconTick, ServerEntity, ClientEntity);

// Internal aliases for clarity in the PredictionTracker types above.
type ServerEntity = Entity;
type ClientEntity = Entity;

impl PredictionTracker {
    /// Register that the server spawned `server_entity` as a result of `client_id` sending a
    /// command which also included a `client_entity` denoting the client's predicted local spawn.
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
        if let Some(mut v) = self.mappings.get_mut(&client_id) {
            v.push(new_entry);
        } else {
            self.mappings.insert(client_id, vec![new_entry]);
        }
    }
    pub(crate) fn get_mappings(
        &self,
        client_id: u64,
        tick: RepliconTick,
    ) -> impl Iterator<Item = &EntityMapping> {
        // let Some(v) = self.mappings.get(&client_id) else {
        //     return None;
        // };
        let v = match self.mappings
            .get(&client_id) {
                Some(v) => v.iter(),
                None => std::iter::empty().into(),
            };


            .map_(std::iter::empty())
            .into_iter()
            .filter(|(entry_tick, _, _)| *entry_tick >= tick)
    }
    /// remove predicted entities in cases where the RepliconTick at which that entity was spawned
    /// has been acked by a client.
    pub(crate) fn cleanup_acked(&mut self, client_id: u64, acked_tick: RepliconTick) {
        let Some(v) = self.tick_map.get_mut(&client_id) else {
            return;
        };
        v.retain(|(tick, server_entity)| {
            if tick.get() > acked_tick.get() {
                // not acked yet, retain it
                return true;
            }
            self.entity_map.remove(&(client_id, *server_entity));
            false
        });
    }
}
