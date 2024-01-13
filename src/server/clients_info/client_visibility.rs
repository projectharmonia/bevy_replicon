use std::iter;

use bevy::{
    prelude::*,
    utils::{EntityHashMap, EntityHashSet},
};

use super::VisibilityPolicy;

pub enum ClientVisibility {
    All {
        just_connected: bool,
    },
    Blacklist {
        list: EntityHashMap<Entity, bool>,
        removed: EntityHashSet<Entity>,
    },
    Whitelist {
        list: EntityHashMap<Entity, bool>,
        removed: EntityHashSet<Entity>,
    },
}

impl ClientVisibility {
    /// Creates a new instance based on the preconfigured policy.
    pub(super) fn new(policy: VisibilityPolicy) -> Self {
        match policy {
            VisibilityPolicy::All => Self::All {
                just_connected: true,
            },
            VisibilityPolicy::Blacklist => Self::Blacklist {
                list: Default::default(),
                removed: Default::default(),
            },
            VisibilityPolicy::Whitelist => Self::Whitelist {
                list: Default::default(),
                removed: Default::default(),
            },
        }
    }

    /// Resets the state to as it was after [`Self::new`].
    pub(super) fn clear(&mut self) {
        match self {
            ClientVisibility::All { just_connected } => *just_connected = true,
            ClientVisibility::Blacklist { list, removed }
            | ClientVisibility::Whitelist { list, removed } => {
                list.clear();
                removed.clear();
            }
        }
    }

    /// Marks all entities as not "just added" and clears the removed.
    ///
    /// Should be called after each tick.
    pub(crate) fn update(&mut self) {
        match self {
            ClientVisibility::All { just_connected } => *just_connected = false,
            ClientVisibility::Blacklist { list, removed }
            | ClientVisibility::Whitelist { list, removed } => {
                for just_added in list.values_mut() {
                    *just_added = false;
                }
                removed.clear();
            }
        }
    }

    pub(super) fn remove_despawned(&mut self, entity: Entity) {
        match self {
            ClientVisibility::All { .. } => (),
            ClientVisibility::Blacklist { list, removed }
            | ClientVisibility::Whitelist { list, removed } => {
                list.remove(&entity);
                removed.remove(&entity);
            }
        }
    }

    pub(super) fn iter_hidden(&self) -> Box<dyn Iterator<Item = Entity> + '_> {
        match self {
            ClientVisibility::All { .. } => Box::new(iter::empty()),
            ClientVisibility::Blacklist { list, .. } => Box::new(
                list.iter()
                    .filter_map(|(&entity, just_added)| just_added.then_some(entity)),
            ),
            ClientVisibility::Whitelist { removed, .. } => Box::new(removed.iter().copied()),
        }
    }

    /// Sets visibility for specific entity.
    ///
    /// # Panics
    ///
    /// Panics if the server plugin was previously configured with [`VisibilityPolicy::All`].
    pub fn set(&mut self, entity: Entity, visibile: bool) {
        match self {
            ClientVisibility::All { .. } => {
                panic!(
                    "visibility shouldn't be set with {:?}",
                    VisibilityPolicy::All
                )
            }
            ClientVisibility::Blacklist { list, removed } => {
                if !visibile {
                    list.insert(entity, true);
                    removed.remove(&entity);
                } else if list.remove(&entity).is_some() {
                    removed.insert(entity);
                }
            }
            ClientVisibility::Whitelist { list, removed } => {
                if visibile {
                    list.insert(entity, true);
                    removed.remove(&entity);
                } else if list.remove(&entity).is_some() {
                    removed.insert(entity);
                }
            }
        }
    }

    /// Gets visibility for specific entity.
    pub fn get(&self, entity: Entity) -> EntityVisibility {
        match self {
            ClientVisibility::All { just_connected } => {
                if *just_connected {
                    EntityVisibility::JustVisible
                } else {
                    EntityVisibility::Visible
                }
            }
            ClientVisibility::Blacklist { list, removed } => {
                if list.contains_key(&entity) {
                    EntityVisibility::Hidden
                } else if removed.contains(&entity) {
                    EntityVisibility::JustVisible
                } else {
                    EntityVisibility::Visible
                }
            }
            ClientVisibility::Whitelist { list, .. } => match list.get(&entity) {
                Some(true) => EntityVisibility::JustVisible,
                Some(false) => EntityVisibility::Visible,
                None => EntityVisibility::Hidden,
            },
        }
    }
}

#[derive(PartialEq)]
pub enum EntityVisibility {
    JustVisible,
    Visible,
    Hidden,
}