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
/// ### Callback for successful predictions
///
/// The client can register a callback for when predictions are successful, to confirm
/// the spawn.
///
/// Typically this is used to remove your game's Prediction marker component, something that might
/// usually include a TTL or timeout, after which the predicted entity would be despawned by
/// your game's misprediction cleanup system.
///
/// You could also insert a component in the callback, and have a fully fledged system do the
/// cleanup later, for example with an `Added<PredictionHit>` query.
///
/// ```rust
/// // on client:
/// # use bevy_replicon::client::NetworkEntityMap;
/// # use bevy::ecs::{component::Component, world::EntityMut, system::ResMut};
/// # #[derive(Component)]
/// # struct Prediction;
/// fn predition_hit_fn(cmd: &mut EntityMut) {
///     // prediction hit: remove any Prediction marker component.
///     // This is specific to your game, replicon does not include a Prediction component.
///     cmd.remove::<Prediction>();
/// }
/// fn client_setup(mut net_entity_map: ResMut<NetworkEntityMap>) {
///     net_entity_map.set_prediction_hit_callback(predition_hit_fn);
/// }
/// ```
///
#[derive(Resource, Debug, Default)]
pub struct PredictionTracker {
    entity_map: HashMap<(u64, ServerEntity), ClientEntity>,
    tick_map: HashMap<u64, Vec<(RepliconTick, ServerEntity)>>,
}

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
        self.entity_map
            .insert((client_id, server_entity), client_entity);

        if let Some(v) = self.tick_map.get_mut(&client_id) {
            v.push((tick, server_entity));
        } else {
            self.tick_map.insert(client_id, vec![(tick, server_entity)]);
        }
    }
    pub(crate) fn get_predicted_entity(
        &self,
        client_id: u64,
        server_entity: Entity,
    ) -> Option<&Entity> {
        self.entity_map.get(&(client_id, server_entity))
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
