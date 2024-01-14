use std::iter;

use bevy::{
    prelude::*,
    utils::{EntityHashMap, EntityHashSet},
};

use super::VisibilityPolicy;

/// Entity visibility settings for a client.
pub enum ClientVisibility {
    All {
        /// Indicates that the client has just connected to the server.
        just_connected: bool,
    },
    Blacklist {
        /// All blacklisted entities and an indicator of whether it is in the queue for deletion at this tick.
        list: EntityHashMap<Entity, bool>,
        /// All entities that were removed from the list on this tick.
        added: EntityHashSet<Entity>,
        /// All entities that were added to the list on this tick.
        removed: EntityHashSet<Entity>,
    },
    Whitelist {
        /// All whitelisted entities and an indicator whether it was added to the list on this tick.
        list: EntityHashMap<Entity, bool>,
        /// All entities that were added to the list on this tick.
        added: EntityHashSet<Entity>,
        /// All entities that were removed from the list on this tick.
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
                added: Default::default(),
                removed: Default::default(),
            },
            VisibilityPolicy::Whitelist => Self::Whitelist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            },
        }
    }

    /// Resets the state to as it was after [`Self::new`].
    pub(super) fn clear(&mut self) {
        match self {
            ClientVisibility::All { just_connected } => *just_connected = true,
            ClientVisibility::Blacklist {
                list,
                added,
                removed,
            }
            | ClientVisibility::Whitelist {
                list,
                added,
                removed,
            } => {
                list.clear();
                added.clear();
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
            ClientVisibility::Blacklist {
                list,
                added,
                removed,
            } => {
                for entity in removed.drain() {
                    list.remove(&entity);
                }
                added.clear();
            }
            ClientVisibility::Whitelist {
                list,
                added,
                removed,
            } => {
                for entity in added.drain() {
                    list.insert(entity, false);
                }
                removed.clear();
            }
        }
    }

    /// Removes a despawned entity tracked by this client.
    pub(super) fn remove_despawned(&mut self, entity: Entity) {
        match self {
            ClientVisibility::All { .. } => (),
            ClientVisibility::Blacklist {
                list,
                added,
                removed,
            } => {
                if let Some(just_removed) = list.get_mut(&entity) {
                    *just_removed = true;
                    removed.remove(&entity);
                    added.remove(&entity);
                }
            }
            ClientVisibility::Whitelist {
                list,
                added,
                removed,
            } => {
                if list.remove(&entity).is_some() {
                    added.remove(&entity);
                    removed.remove(&entity);
                }
            }
        }
    }

    /// Returns an iterator of entities that lost visibility at this tick.
    pub(super) fn iter_lost_visibility(&self) -> Box<dyn Iterator<Item = Entity> + '_> {
        match self {
            ClientVisibility::All { .. } => Box::new(iter::empty()),
            ClientVisibility::Blacklist { added, .. } => Box::new(added.iter().copied()),
            ClientVisibility::Whitelist { removed, .. } => Box::new(removed.iter().copied()),
        }
    }

    /// Sets visibility for specific entity.
    ///
    /// Does nothing if visibility policy for the server plugin is set to [`VisibilityPolicy::All`].
    pub fn set(&mut self, entity: Entity, visibile: bool) {
        match self {
            ClientVisibility::All { .. } => {
                if visibile {
                    debug!(
                        "ignoring visibility enable due to {:?}",
                        VisibilityPolicy::All
                    );
                } else {
                    warn!(
                        "ignoring visibility disable due to {:?}",
                        VisibilityPolicy::All
                    );
                }
            }
            ClientVisibility::Blacklist {
                list,
                added,
                removed,
            } => {
                if !visibile {
                    if list.insert(entity, false).is_none() {
                        removed.remove(&entity);
                        added.insert(entity);
                    }
                } else if list.remove(&entity).is_some() {
                    removed.insert(entity);
                    added.remove(&entity);
                }
            }
            ClientVisibility::Whitelist {
                list,
                added,
                removed,
            } => {
                if visibile {
                    if list.insert(entity, true).is_none() {
                        removed.remove(&entity);
                        added.insert(entity);
                    }
                } else if list.remove(&entity).is_some() {
                    removed.insert(entity);
                    added.remove(&entity);
                }
            }
        }
    }

    /// Gets visibility for specific entity.
    pub fn get(&self, entity: Entity) -> EntityVisibility {
        match self {
            ClientVisibility::All { just_connected } => {
                if *just_connected {
                    EntityVisibility::Gained
                } else {
                    EntityVisibility::Maintained
                }
            }
            ClientVisibility::Blacklist { list, .. } => match list.get(&entity) {
                Some(true) => EntityVisibility::Gained,
                Some(false) => EntityVisibility::None,
                None => EntityVisibility::Maintained,
            },
            ClientVisibility::Whitelist { list, .. } => match list.get(&entity) {
                Some(true) => EntityVisibility::Gained,
                Some(false) => EntityVisibility::Maintained,
                None => EntityVisibility::None,
            },
        }
    }
}

#[derive(PartialEq)]
pub enum EntityVisibility {
    Gained,
    Maintained,
    None,
}
