use bevy::{
    prelude::*,
    utils::{EntityHashMap, EntityHashSet},
};

use super::VisibilityPolicy;

/// Entity visibility settings for a client.
pub struct ClientVisibility(VisibilityFilter);

impl ClientVisibility {
    /// Creates a new instance based on the preconfigured policy.
    pub(super) fn new(policy: VisibilityPolicy) -> Self {
        match policy {
            VisibilityPolicy::All => Self(VisibilityFilter::All {
                just_connected: true,
            }),
            VisibilityPolicy::Blacklist => Self(VisibilityFilter::Blacklist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
            VisibilityPolicy::Whitelist => Self(VisibilityFilter::Whitelist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
        }
    }

    /// Resets the state to as it was after [`Self::new`].
    pub(super) fn clear(&mut self) {
        match &mut self.0 {
            VisibilityFilter::All { just_connected } => *just_connected = true,
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            }
            | VisibilityFilter::Whitelist {
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
        match &mut self.0 {
            VisibilityFilter::All { just_connected } => *just_connected = false,
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                for entity in removed.drain() {
                    list.remove(&entity);
                }
                added.clear();
            }
            VisibilityFilter::Whitelist {
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
        match &mut self.0 {
            VisibilityFilter::All { .. } => (),
            VisibilityFilter::Blacklist {
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
            VisibilityFilter::Whitelist {
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
    pub(super) fn iter_lost_visibility(&self) -> impl Iterator<Item = Entity> + '_ {
        match &self.0 {
            VisibilityFilter::All { .. } => VisibilityLostIter::AllVisible,
            VisibilityFilter::Blacklist { added, .. } => {
                VisibilityLostIter::Lost(added.iter().copied())
            }
            VisibilityFilter::Whitelist { removed, .. } => {
                VisibilityLostIter::Lost(removed.iter().copied())
            }
        }
    }

    /// Sets visibility for specific entity.
    ///
    /// Does nothing if visibility policy for the server plugin is set to [`VisibilityPolicy::All`].
    pub fn set_visible(&mut self, entity: Entity, visibile: bool) {
        match &mut self.0 {
            VisibilityFilter::All { .. } => {
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
            VisibilityFilter::Blacklist {
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
            VisibilityFilter::Whitelist {
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
    pub fn is_visible(&self, entity: Entity) -> bool {
        match &self.0 {
            VisibilityFilter::All { .. } => true,
            VisibilityFilter::Blacklist { list, .. } => !list.contains_key(&entity),
            VisibilityFilter::Whitelist { list, .. } => list.contains_key(&entity),
        }
    }

    /// Gets visibility with change information included for specific entity.
    pub(crate) fn get_info(&self, entity: Entity) -> VisibilityInfo {
        match &self.0 {
            VisibilityFilter::All { just_connected } => {
                if *just_connected {
                    VisibilityInfo::Gained
                } else {
                    VisibilityInfo::Maintained
                }
            }
            VisibilityFilter::Blacklist { list, .. } => match list.get(&entity) {
                Some(true) => VisibilityInfo::Gained,
                Some(false) => VisibilityInfo::None,
                None => VisibilityInfo::Maintained,
            },
            VisibilityFilter::Whitelist { list, .. } => match list.get(&entity) {
                Some(true) => VisibilityInfo::Gained,
                Some(false) => VisibilityInfo::Maintained,
                None => VisibilityInfo::None,
            },
        }
    }
}

/// Since enums can't have private fields, it's wrapped into [`ClientVisibility`] to make fields private, while keeping the struct public.
enum VisibilityFilter {
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

#[derive(PartialEq)]
pub(crate) enum VisibilityInfo {
    Gained,
    Maintained,
    None,
}

enum VisibilityLostIter<T> {
    AllVisible,
    Lost(T),
}

impl<T: Iterator> Iterator for VisibilityLostIter<T> {
    type Item = T::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            VisibilityLostIter::AllVisible => None,
            VisibilityLostIter::Lost(entities) => entities.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            VisibilityLostIter::AllVisible => (0, Some(0)),
            VisibilityLostIter::Lost(entities) => entities.size_hint(),
        }
    }
}

impl<T: ExactSizeIterator> ExactSizeIterator for VisibilityLostIter<T> {}
