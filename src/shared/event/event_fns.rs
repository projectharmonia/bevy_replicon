use alloc::vec::Vec;
use core::{
    any::{self, TypeId},
    mem,
};

use bevy::prelude::*;
use bytes::Bytes;

/// Type-erased version of [`EventFns`].
///
/// Stored inside events after their creation.
#[derive(Clone, Copy)]
pub(super) struct UntypedEventFns {
    serialize_ctx_id: TypeId,
    serialize_ctx_name: &'static str,
    deserialize_ctx_id: TypeId,
    deserialize_ctx_name: &'static str,
    event_id: TypeId,
    event_name: &'static str,
    inner_id: TypeId,
    inner_name: &'static str,

    outer_serialize: unsafe fn(),
    outer_deserialize: unsafe fn(),
    serialize: unsafe fn(),
    deserialize: unsafe fn(),
}

impl UntypedEventFns {
    /// Restores the original [`EventFns`] from which this type was created.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the function is called with the same generics with which this instance was created.
    pub(super) unsafe fn typed<S, D, E: 'static, I: 'static>(self) -> EventFns<S, D, E, I> {
        // `TypeId` can only be obtained for `'static` types, but we can't impose this requirement because our context has a lifetime.
        // So, we use the `typeid` crate for non-static `TypeId`, as we don't care about the lifetime and only need to check the type.
        // This crate is already used by `erased_serde`, so we don't add an extra dependency.
        debug_assert_eq!(
            self.serialize_ctx_id,
            typeid::of::<S>(),
            "trying to call event functions with serialize context `{}`, but they were created with `{}`",
            any::type_name::<S>(),
            self.serialize_ctx_name,
        );
        debug_assert_eq!(
            self.deserialize_ctx_id,
            typeid::of::<D>(),
            "trying to call event functions with deserialize context `{}`, but they were created with `{}`",
            any::type_name::<D>(),
            self.deserialize_ctx_name,
        );
        debug_assert_eq!(
            self.event_id,
            TypeId::of::<E>(),
            "trying to call event functions with event `{}`, but they were created with `{}`",
            any::type_name::<E>(),
            self.event_name,
        );
        debug_assert_eq!(
            self.inner_id,
            TypeId::of::<I>(),
            "trying to call event functions with inner type `{}`, but they were created with `{}`",
            any::type_name::<I>(),
            self.inner_name,
        );

        EventFns {
            outer_serialize: unsafe {
                mem::transmute::<unsafe fn(), OuterSerializeFn<S, E, I>>(self.outer_serialize)
            },
            outer_deserialize: unsafe {
                mem::transmute::<unsafe fn(), OuterDeserializeFn<D, E, I>>(self.outer_deserialize)
            },
            serialize: unsafe {
                mem::transmute::<unsafe fn(), EventSerializeFn<S, I>>(self.serialize)
            },
            deserialize: unsafe {
                mem::transmute::<unsafe fn(), EventDeserializeFn<D, I>>(self.deserialize)
            },
        }
    }
}

impl<S, D, E: 'static, I: 'static> From<EventFns<S, D, E, I>> for UntypedEventFns {
    fn from(value: EventFns<S, D, E, I>) -> Self {
        // SAFETY: these functions won't be called until the type is restored.
        Self {
            serialize_ctx_id: typeid::of::<S>(),
            serialize_ctx_name: any::type_name::<S>(),
            deserialize_ctx_id: typeid::of::<D>(),
            deserialize_ctx_name: any::type_name::<D>(),
            event_id: TypeId::of::<E>(),
            event_name: any::type_name::<E>(),
            inner_id: TypeId::of::<I>(),
            inner_name: any::type_name::<I>(),
            outer_serialize: unsafe {
                mem::transmute::<OuterSerializeFn<S, E, I>, unsafe fn()>(value.outer_serialize)
            },
            outer_deserialize: unsafe {
                mem::transmute::<OuterDeserializeFn<D, E, I>, unsafe fn()>(value.outer_deserialize)
            },
            serialize: unsafe {
                mem::transmute::<EventSerializeFn<S, I>, unsafe fn()>(value.serialize)
            },
            deserialize: unsafe {
                mem::transmute::<EventDeserializeFn<D, I>, unsafe fn()>(value.deserialize)
            },
        }
    }
}

/// Serialization and deserialization functions for an event.
///
/// For triggers, we want to allow users to customize these functions, but it would be inconvenient
/// to write serialization and deserialization logic for trigger target entities every time.
/// Since closures can't be used, we provide outer functions that accept regular serialization functions.
/// By default, these outer functions simply call the inner function, but they can be overridden
/// to write common serde logic.
pub(super) struct EventFns<S, D, E, I = E> {
    outer_serialize: OuterSerializeFn<S, E, I>,
    outer_deserialize: OuterDeserializeFn<D, E, I>,
    serialize: EventSerializeFn<S, I>,
    deserialize: EventDeserializeFn<D, I>,
}

impl<S, D, E> EventFns<S, D, E, E> {
    /// Creates a new instance with default outer functions.
    pub(super) fn new(
        serialize: EventSerializeFn<S, E>,
        deserialize: EventDeserializeFn<D, E>,
    ) -> Self {
        Self {
            outer_serialize: default_outer_serialize::<S, E>,
            outer_deserialize: default_outer_deserialize::<D, E>,
            serialize,
            deserialize,
        }
    }
}

impl<S, D, E, I> EventFns<S, D, E, I> {
    /// Overrides current outer functions.
    pub(super) fn with_outer<T>(
        self,
        outer_serialize: OuterSerializeFn<S, T, I>,
        outer_deserialize: OuterDeserializeFn<D, T, I>,
    ) -> EventFns<S, D, T, I> {
        EventFns {
            outer_serialize,
            outer_deserialize,
            serialize: self.serialize,
            deserialize: self.deserialize,
        }
    }

    pub(super) fn serialize(self, ctx: &mut S, event: &E, message: &mut Vec<u8>) -> Result<()> {
        (self.outer_serialize)(ctx, event, message, self.serialize)
    }

    pub(super) fn deserialize(self, ctx: &mut D, message: &mut Bytes) -> Result<E> {
        (self.outer_deserialize)(ctx, message, self.deserialize)
    }
}

fn default_outer_serialize<C, E>(
    ctx: &mut C,
    event: &E,
    message: &mut Vec<u8>,
    serialize: EventSerializeFn<C, E>,
) -> Result<()> {
    (serialize)(ctx, event, message)
}

fn default_outer_deserialize<C, E>(
    ctx: &mut C,
    message: &mut Bytes,
    deserialize: EventDeserializeFn<C, E>,
) -> Result<E> {
    (deserialize)(ctx, message)
}

/// Signature of event serialization functions.
pub type EventSerializeFn<C, E> = fn(&mut C, &E, &mut Vec<u8>) -> Result<()>;

/// Signature of event deserialization functions.
pub type EventDeserializeFn<C, E> = fn(&mut C, &mut Bytes) -> Result<E>;

/// Signature of outer serialization functions.
pub(super) type OuterSerializeFn<C, E, I> =
    fn(&mut C, &E, &mut Vec<u8>, EventSerializeFn<C, I>) -> Result<()>;

/// Signature of outer deserialization functions.
pub(super) type OuterDeserializeFn<C, E, I> =
    fn(&mut C, &mut Bytes, EventDeserializeFn<C, I>) -> Result<E>;
