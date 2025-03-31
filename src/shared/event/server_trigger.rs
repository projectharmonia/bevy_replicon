use core::any;

use bevy::{ecs::entity::MapEntities, prelude::*, ptr::PtrMut};
use bytes::Bytes;
use log::debug;
use serde::{Serialize, de::DeserializeOwned};

use super::{
    ctx::{ClientReceiveCtx, ServerSendCtx},
    event_fns::{EventDeserializeFn, EventFns, EventSerializeFn},
    remote_event_registry::RemoteEventRegistry,
    server_event::{self, ServerEvent, ToClients},
    trigger::{RemoteTargets, RemoteTrigger},
};
use crate::shared::{backend::replicon_channels::Channel, entity_serde, postcard_utils};

/// An extension trait for [`App`] for creating server triggers.
///
/// See also [`ServerTriggerExt`].
pub trait ServerTriggerAppExt {
    /// Registers a remote event that can be triggered using [`ServerTriggerExt::server_trigger`].
    ///
    /// After triggering [`ToClients<E>`] event on the server, `E` event will be triggered on clients.
    ///
    /// If [`ClientEventPlugin`](crate::client::event::ClientEventPlugin) is enabled and
    /// [`SERVER`](crate::shared::SERVER) is a recipient of the event (not to be confused with trigger target),
    /// then `E` event will be emitted on the server as well.
    ///
    /// See also [`Self::add_server_trigger_with`] and the [corresponding section](../index.html#from-server-to-client)
    /// from the quick start guide.
    fn add_server_trigger<E: Event + Serialize + DeserializeOwned>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_server_trigger_with(
            channel,
            server_event::default_serialize::<E>,
            server_event::default_deserialize::<E>,
        )
    }

    /// Same as [`Self::add_server_trigger`], but additionally maps client entities to server inside the event before receiving.
    ///
    /// Always use it for events that contain entities.
    fn add_mapped_server_trigger<E: Event + Serialize + DeserializeOwned + MapEntities>(
        &mut self,
        channel: Channel,
    ) -> &mut Self {
        self.add_server_trigger_with(
            channel,
            server_event::default_serialize::<E>,
            server_event::default_deserialize_mapped::<E>,
        )
    }

    /// Same as [`Self::add_server_trigger`], but uses the specified functions for serialization and deserialization.
    ///
    /// See also [`ServerEventAppExt::add_server_event_with`](super::server_event::ServerEventAppExt::add_server_event_with).
    fn add_server_trigger_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ServerSendCtx, E>,
        deserialize: EventDeserializeFn<ClientReceiveCtx, E>,
    ) -> &mut Self;
}

impl ServerTriggerAppExt for App {
    fn add_server_trigger_with<E: Event>(
        &mut self,
        channel: Channel,
        serialize: EventSerializeFn<ServerSendCtx, E>,
        deserialize: EventDeserializeFn<ClientReceiveCtx, E>,
    ) -> &mut Self {
        debug!("registering trigger `{}`", any::type_name::<E>());

        let event_fns = EventFns::new(serialize, deserialize)
            .with_outer(trigger_serialize, trigger_deserialize);
        let trigger = ServerTrigger::new(self, channel, event_fns);
        let mut event_registry = self.world_mut().resource_mut::<RemoteEventRegistry>();
        event_registry.register_server_trigger(trigger);

        self
    }
}

/// Small abstraction on top of [`ServerEvent`] that stores a function to trigger them.
pub(crate) struct ServerTrigger {
    trigger: TriggerFn,
    event: ServerEvent,
}

impl ServerTrigger {
    fn new<E: Event>(
        app: &mut App,
        channel: Channel,
        event_fns: EventFns<ServerSendCtx, ClientReceiveCtx, RemoteTrigger<E>, E>,
    ) -> Self {
        let event = ServerEvent::new(app, channel, event_fns);
        Self {
            trigger: Self::trigger_typed::<E>,
            event,
        }
    }

    pub(crate) fn trigger(&self, commands: &mut Commands, events: PtrMut) {
        unsafe {
            (self.trigger)(commands, events);
        }
    }

    /// Drains received [`RemoteTrigger<E>`] events and triggers them as `E`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `events` is [`Events<RemoteTrigger<E>>`]
    /// and this instance was created for `E`.
    unsafe fn trigger_typed<E: Event>(commands: &mut Commands, events: PtrMut) {
        let events: &mut Events<RemoteTrigger<E>> = unsafe { events.deref_mut() };
        for trigger in events.drain() {
            debug!("triggering `{}`", any::type_name::<E>());
            commands.trigger_targets(trigger.event, trigger.targets);
        }
    }

    pub(crate) fn event(&self) -> &ServerEvent {
        &self.event
    }

    pub(super) fn event_mut(&mut self) -> &mut ServerEvent {
        &mut self.event
    }
}

/// Signature of server trigger functions.
type TriggerFn = unsafe fn(&mut Commands, PtrMut);

/// Serializes targets for [`RemoteTrigger`] and delegates the event
/// serialiaztion to `serialize`.
///
/// Used as outer function for [`EventFns`].
fn trigger_serialize<'a, E>(
    ctx: &mut ServerSendCtx<'a>,
    trigger: &RemoteTrigger<E>,
    message: &mut Vec<u8>,
    serialize: EventSerializeFn<ServerSendCtx<'a>, E>,
) -> Result<()> {
    postcard_utils::to_extend_mut(&trigger.targets.len(), message)?;
    for &entity in &trigger.targets {
        entity_serde::serialize_entity(message, entity)?;
    }

    (serialize)(ctx, &trigger.event, message)
}

/// Deserializes targets for [`RemoteTrigger`] and delegates the event
/// deserialiaztion to `deserialize`.
///
/// Used as outer function for [`EventFns`].
fn trigger_deserialize<'a, E>(
    ctx: &mut ClientReceiveCtx<'a>,
    message: &mut Bytes,
    deserialize: EventDeserializeFn<ClientReceiveCtx<'a>, E>,
) -> Result<RemoteTrigger<E>> {
    let len = postcard_utils::from_buf(message)?;
    let mut targets = Vec::with_capacity(len);
    for _ in 0..len {
        let entity = entity_serde::deserialize_entity(message)?;
        targets.push(ctx.get_mapped(entity));
    }

    let event = (deserialize)(ctx, message)?;

    Ok(RemoteTrigger { event, targets })
}

/// Extension trait for triggering server events.
///
/// See also [`ServerTriggerAppExt`].
pub trait ServerTriggerExt {
    /// Like [`Commands::trigger`], but triggers `E` on server and locally
    /// if [`SERVER`](crate::shared::SERVER) is a recipient of the event).
    fn server_trigger(&mut self, event: ToClients<impl Event>);

    /// Like [`Self::server_trigger`], but allows you to specify target entities, similar to
    /// [`Commands::trigger_targets`].
    fn server_trigger_targets(&mut self, event: ToClients<impl Event>, targets: impl RemoteTargets);
}

impl ServerTriggerExt for Commands<'_, '_> {
    fn server_trigger(&mut self, event: ToClients<impl Event>) {
        self.server_trigger_targets(event, []);
    }

    fn server_trigger_targets(
        &mut self,
        event: ToClients<impl Event>,
        targets: impl RemoteTargets,
    ) {
        self.send_event(ToClients {
            mode: event.mode,
            event: RemoteTrigger {
                event: event.event,
                targets: targets.into_entities(),
            },
        });
    }
}

impl ServerTriggerExt for World {
    fn server_trigger(&mut self, event: ToClients<impl Event>) {
        self.server_trigger_targets(event, []);
    }

    fn server_trigger_targets(
        &mut self,
        event: ToClients<impl Event>,
        targets: impl RemoteTargets,
    ) {
        self.send_event(ToClients {
            mode: event.mode,
            event: RemoteTrigger {
                event: event.event,
                targets: targets.into_entities(),
            },
        });
    }
}
