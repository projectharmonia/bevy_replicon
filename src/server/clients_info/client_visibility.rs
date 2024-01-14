use bevy::{
    prelude::*,
    utils::{EntityHashMap, EntityHashSet},
};

use super::VisibilityPolicy;

/// Entity visibility settings for a client.
pub struct ClientVisibility {
    filter: VisibilityFilter,
    entity_state: EntityState,
}

impl ClientVisibility {
    /// Creates a new instance based on the preconfigured policy.
    pub(super) fn new(policy: VisibilityPolicy) -> Self {
        match policy {
            VisibilityPolicy::All => Self::with_filter(VisibilityFilter::All {
                just_connected: true,
            }),
            VisibilityPolicy::Blacklist => Self::with_filter(VisibilityFilter::Blacklist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
            VisibilityPolicy::Whitelist => Self::with_filter(VisibilityFilter::Whitelist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
        }
    }

    /// Creates a new instance with a specific filter.
    fn with_filter(filter: VisibilityFilter) -> Self {
        Self {
            filter,
            entity_state: Default::default(),
        }
    }

    /// Resets the filter state to as it was after [`Self::new`].
    ///
    /// `entity_state` remains untouched.
    pub(super) fn clear(&mut self) {
        match &mut self.filter {
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
        match &mut self.filter {
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
        match &mut self.filter {
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

    /// Returns an iterator of entities the client lost visibility of this tick.
    pub(super) fn iter_lost_visibility(&self) -> impl Iterator<Item = Entity> + '_ {
        match &self.filter {
            VisibilityFilter::All { .. } => VisibilityLostIter::AllVisible,
            VisibilityFilter::Blacklist { added, .. } => {
                VisibilityLostIter::Lost(added.iter().copied())
            }
            VisibilityFilter::Whitelist { removed, .. } => {
                VisibilityLostIter::Lost(removed.iter().copied())
            }
        }
    }

    /// Sets visibility for a specific entity.
    ///
    /// Does nothing if the visibility policy for the server plugin is set to [`VisibilityPolicy::All`].
    pub fn set_visibility(&mut self, entity: Entity, visibile: bool) {
        match &mut self.filter {
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
                } else if let Some(just_removed) = list.get_mut(&entity) {
                    *just_removed = true;
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

    /// Gets visibility for a specific entity.
    pub fn is_visible(&self, entity: Entity) -> bool {
        match &self.filter {
            VisibilityFilter::All { .. } => true,
            VisibilityFilter::Blacklist { list, .. } => !list.contains_key(&entity),
            VisibilityFilter::Whitelist { list, .. } => list.contains_key(&entity),
        }
    }

    /// Reads entity visibility state for specific entity.
    ///
    /// Can be obtained later from [`Self::entity_state`].
    pub(crate) fn read_entity_state(&mut self, entity: Entity) {
        match &mut self.filter {
            VisibilityFilter::All { just_connected } => {
                if *just_connected {
                    self.entity_state = EntityState::JustVisible
                } else {
                    self.entity_state = EntityState::Visible
                }
            }
            VisibilityFilter::Blacklist { list, .. } => match list.get(&entity) {
                Some(true) => self.entity_state = EntityState::JustVisible,
                Some(false) => self.entity_state = EntityState::Hidden,
                None => self.entity_state = EntityState::Visible,
            },
            VisibilityFilter::Whitelist { list, .. } => match list.get(&entity) {
                Some(true) => self.entity_state = EntityState::JustVisible,
                Some(false) => self.entity_state = EntityState::Visible,
                None => self.entity_state = EntityState::Hidden,
            },
        }
    }

    /// Returns state obtained from last call of [`Self::read_entity_state`].
    pub(crate) fn entity_state(&self) -> EntityState {
        self.entity_state
    }
}

/// Filter for [`ClientVisibility`] based on [`VisibilityPolicy`].
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

/// Visibility state for an entity.
#[derive(PartialEq, Default, Clone, Copy)]
pub(crate) enum EntityState {
    #[default]
    None,
    Hidden,
    Visible,
    JustVisible,
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
