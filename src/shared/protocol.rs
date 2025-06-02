use core::{
    any,
    hash::{BuildHasher, Hash, Hasher},
};

use bevy::{
    platform::hash::{DefaultHasher, FixedHasher},
    prelude::*,
};
use serde::{Deserialize, Serialize};

/// Hashes all protocol registrations to calculate [`ProtocolHash`].
///
/// The hash is computed using type names and their use in the protocol. We can't detect
/// things like different function registrations because there is no portable way of
/// doing so. But this at least prevents users from common mistakes such as using
/// different registration order or accidentally registering different things on
/// the client and server, which are very difficult to debug.
///
/// You can include custom data (e.g., a game version) via [`Self::add_custom`].
///
/// Only available during the [`Plugin::build`] stage. Computes [`ProtocolHash`] resource.
#[derive(Resource)]
pub struct ProtocolHasher(DefaultHasher);

impl ProtocolHasher {
    /// Adds custom data to the protocol hash calculation.
    ///
    /// # Examples
    ///
    /// Include a game version.
    ///
    /// ```
    /// use bevy::prelude::*;
    /// use bevy_replicon::prelude::*;
    /// let mut app = App::new();
    /// app.add_plugins((MinimalPlugins, RepliconPlugins));
    ///
    /// // Should be called before `app.run()` or `app.finish()`.
    /// // Can also be done inside your game's plugin.
    /// let mut hasher = app.world_mut().resource_mut::<ProtocolHasher>();
    /// hasher.add_custom(env!("CARGO_PKG_VERSION"));
    /// ```
    pub fn add_custom<T: Hash>(&mut self, value: T) {
        value.hash(&mut self.0);
    }

    pub(crate) fn replicate<R>(&mut self, priority: usize) {
        self.hash::<R>(ProtocolPart::Replicate { priority });
    }

    pub(crate) fn replicate_bundle<B>(&mut self) {
        self.hash::<B>(ProtocolPart::ReplicateBundle);
    }

    pub(crate) fn add_client_event<E>(&mut self) {
        self.hash::<E>(ProtocolPart::ClientEvent);
    }

    pub(crate) fn add_client_trigger<E>(&mut self) {
        self.hash::<E>(ProtocolPart::ClientTrigger);
    }

    pub(crate) fn add_server_event<E>(&mut self) {
        self.hash::<E>(ProtocolPart::ServerEvent);
    }

    pub(crate) fn add_server_trigger<E>(&mut self) {
        self.hash::<E>(ProtocolPart::ServerTrigger);
    }

    fn hash<T>(&mut self, part: ProtocolPart) {
        part.hash(&mut self.0);
        any::type_name::<T>().hash(&mut self.0);
    }

    pub(crate) fn finish(self) -> ProtocolHash {
        ProtocolHash(self.0.finish())
    }
}

impl Default for ProtocolHasher {
    fn default() -> Self {
        Self(FixedHasher.build_hasher())
    }
}

/// Part of protocol registration.
///
/// Needed to distinguish between different registrations for the same type.
/// For example, the same type could be used for a client and a server event.
#[derive(Hash)]
enum ProtocolPart {
    Replicate { priority: usize },
    ReplicateBundle,
    ClientEvent,
    ClientTrigger,
    ServerEvent,
    ServerTrigger,
}

/// Hash of all registered events and replication rules.
///
/// Used to verify compatibility between client and server.
///
/// Calculated by [`ProtocolHasher`] and available only after [`Plugin::finish`].
#[derive(Resource, Event, Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolHash(u64);

/// A server trigger to notify client for the protocol mismatch.
///
/// Registered and sent only if [`RepliconSharedPlugin::auth_method`](super::RepliconSharedPlugin::auth_method)
/// set to [`AuthMethod::ProtocolCheck`](super::AuthMethod::ProtocolCheck). The server will immediately
/// disconnect after sending it, so there is no delivery guarantee.
#[derive(Event, Serialize, Deserialize)]
pub struct ProtocolMismatch;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        assert_eq!(
            ProtocolHasher::default().finish(),
            ProtocolHasher::default().finish()
        );
    }

    #[test]
    fn wrong_order() {
        let mut hasher1 = ProtocolHasher::default();
        hasher1.replicate::<StructA>(1);
        hasher1.replicate::<StructB>(1);

        let mut hasher2 = ProtocolHasher::default();
        hasher2.replicate::<StructB>(1);
        hasher2.replicate::<StructA>(1);

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn wrong_priority() {
        let mut hasher1 = ProtocolHasher::default();
        hasher1.replicate::<StructA>(1);

        let mut hasher2 = ProtocolHasher::default();
        hasher2.replicate::<StructA>(0);

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn different_parts() {
        let mut hasher1 = ProtocolHasher::default();
        hasher1.add_server_event::<StructA>();

        let mut hasher2 = ProtocolHasher::default();
        hasher2.add_client_event::<StructA>();

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn mismatch() {
        let mut hasher1 = ProtocolHasher::default();
        let mut hasher2 = ProtocolHasher::default();

        for hasher in [&mut hasher1, &mut hasher2] {
            hasher.replicate::<StructA>(1);
            hasher.add_server_event::<StructB>();
            hasher.add_server_trigger::<StructC>();
            hasher.add_client_event::<StructB>();
            hasher.add_client_trigger::<StructC>();
        }
        hasher1.add_custom(0);
        hasher2.add_custom(1);

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn full_match() {
        let mut hasher1 = ProtocolHasher::default();
        let mut hasher2 = ProtocolHasher::default();

        for hasher in [&mut hasher1, &mut hasher2] {
            hasher.replicate::<StructA>(1);
            hasher.add_server_event::<StructB>();
            hasher.add_server_trigger::<StructC>();
            hasher.add_client_event::<StructB>();
            hasher.add_client_trigger::<StructC>();
            hasher.add_custom(0);
        }

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    struct StructA;
    struct StructB;
    struct StructC;
}
